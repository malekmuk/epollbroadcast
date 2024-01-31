use std::collections::HashMap;
use std::net::{TcpListener, TcpStream};
use std::io::{Error, ErrorKind, Read, Result, Write};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicUsize, Ordering};
use structopt::StructOpt;

const MAX_EVENTS: i32 = 256;
const BUFFER_SIZE: usize = 256;

static TOTAL_BYTES_SENT: AtomicUsize = AtomicUsize::new(0);

#[derive(StructOpt, Debug)]
#[structopt(name = "epollserver")]
struct Opt {
    #[structopt(short, long, default_value = "9090")]
    port: u16
}

struct ClientState {
    off: usize,
    needle: usize, /* position of last \n */
    buf: Box<[u8; BUFFER_SIZE]>,
    stream: TcpStream,
}

impl ClientState {
    pub fn with_stream(stream: TcpStream) -> ClientState {
        ClientState {
            off: 0, /* index of last u8 in buf if buf has no \n */
            needle: 0, /* index of last \n in buf */
            buf: Box::new([0; BUFFER_SIZE]),
            stream,
        }
    }
}

// tries to write orators buffer to every client connected, does not try again
// if write fails.
//
// returns total number of bytes written across all clients
fn broadcast_message(orator: &mut ClientState, clients: &mut HashMap<i32, ClientState>) -> usize {
    let ofd = orator.stream.as_raw_fd();
    let message = &orator.buf[0..orator.needle];
    let mut bytes = 0;

    for (cfd, client) in clients.iter_mut() {
        if *cfd != ofd {
            match client.stream.write(message) {
                Ok(n) => bytes += n,
                Err(_e) => /*eprintln!("write error (fd = {}): {e}", *cfd)*/ {},
            }
        }
    }

    // if there are left over bytes past the needle, shift them to the 
    // beginning of the buffer for next read, this way writes always start at index 0
    if orator.needle < orator.off {
        orator.off -= orator.needle;
        for i in 0..orator.off {
            orator.buf[i] = orator.buf[orator.needle];
            orator.needle += 1;
        }
    } else {
        orator.off = 0;
    }
    orator.needle = 0;

    bytes
}

/* returns true if message should be broadcasted */
fn check_message(client: &mut ClientState, bytes: usize) -> bool {
    for i in (0..client.off + bytes).rev() {
        if client.buf[i] == b'\n' {
            client.needle = i + 1;
            break;
        }
    }

    client.off += bytes;
    if client.needle > 0 {
        return true;
    }

    false
}

fn handle_client(cfd: i32, clients: &mut HashMap<i32, ClientState>) -> Result<()> {
    let client = clients.get_mut(&cfd).unwrap() as *mut ClientState;
    
    match unsafe { (*client).stream.read(&mut (*client).buf[(*client).off..BUFFER_SIZE]) } {
        Ok(bytes) => {
            if bytes == 0 { 
                return Err(Error::from(ErrorKind::ConnectionAborted)); 
            }

            unsafe {
                if check_message(&mut *client, bytes) {
                    let sent = broadcast_message(&mut *client, clients);
                    TOTAL_BYTES_SENT.fetch_add(sent, Ordering::Relaxed);
                    println!("sent {:?} bytes", TOTAL_BYTES_SENT);
                }
            }

            Ok(())
        },
        Err(e) => {
            match e.kind() {
                ErrorKind::WouldBlock => Ok(()),
                _ => Err(e)
            }
        }
    }
}

fn remove_client(epfd: i32, cfd: i32, clients: &mut HashMap<i32, ClientState>) {
    unsafe { libc::epoll_ctl(epfd, libc::EPOLL_CTL_DEL, cfd, std::ptr::null_mut()); }
    clients.remove(&cfd);
    println!("removed client {}", cfd);
}

fn accept_client(epfd: i32, listener: &TcpListener) -> Result<TcpStream> {
    let (stream, _) = match listener.accept() {
        Ok(s) => s,
        Err(e) => return Err(e)
    };

    if let Err(e) = stream.set_nonblocking(true) {
        return Err(e);
    }
    let fd = stream.as_raw_fd();
    println!("accepted a client (fd = {})", fd);

    let mut e = libc::epoll_event {
        events: libc::EPOLLIN as u32,
        u64: fd as u64
    };

    let ret = unsafe { libc::epoll_ctl(epfd, libc::EPOLL_CTL_ADD, fd, &mut e) };
    if ret < 0 {
        eprintln!("failed to add client to epoll");
        return Err(Error::last_os_error());
    }

    Ok(stream)
}

fn await_clients(listener: TcpListener, epfd: i32, events: *mut libc::epoll_event) {
    let mut clients: HashMap<i32, ClientState> = HashMap::new();
    let sockfd = listener.as_raw_fd() as u64;

    loop {
        let ready = unsafe { libc::epoll_wait(epfd, events, MAX_EVENTS, -1) };
        if ready < 0 {
            eprintln!("epoll_wait error: {}", Error::last_os_error());
            continue;
        }

        for i in 0..ready as isize {
            if let Some(event) = unsafe { events.offset(i).as_ref() } {
                if event.u64 == sockfd {
                    if let Ok(stream) = accept_client(epfd, &listener) {
                        clients.insert(stream.as_raw_fd(), ClientState::with_stream(stream));
                    }
                } else {
                    if let Err(_e) = handle_client(event.u64 as i32, &mut clients) {
                        remove_client(epfd, event.u64 as i32, &mut clients)
                    }
                }
            }
        }
    }
}

fn epoll_init(sockfd: i32) -> Result<i32> {
    unsafe {
        let epfd = libc::epoll_create1(0);

        if epfd >= 0 {
            let mut e = libc::epoll_event {
                events: libc::EPOLLIN as u32,
                u64: sockfd as u64
            };
            
            if libc::epoll_ctl(epfd, libc::EPOLL_CTL_ADD, sockfd, &mut e) == 0 {
                return Ok(epfd);
            } else {
                let errmsg = format!("epoll_ctl failed to add server fd {} -- {}", sockfd, Error::last_os_error());
                return Err(Error::other(errmsg));
            }
        }
    }
    
    let errmsg = format!("epoll_create1 failed -- {}", Error::last_os_error());
    Err(Error::other(errmsg))
}

fn main() -> Result<()> {
    let opt = Opt::from_args();
    let addr = format!("localhost:{}", opt.port);
    let listener = TcpListener::bind(addr)?;

    println!("epoll server listening on port {}...\n", opt.port);

    let epfd = epoll_init(listener.as_raw_fd())?;
    let mut events = Vec::with_capacity(MAX_EVENTS as usize);
    await_clients(listener, epfd, events.as_mut_ptr());

    Ok(())
}