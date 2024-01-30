use std::borrow::{Borrow, BorrowMut};
use std::collections::HashMap;
use std::net::{TcpListener, TcpStream, Shutdown};
use std::io::{Error, ErrorKind, Read, Result, Write};
use std::os::fd::AsRawFd;
use structopt::StructOpt;

const MAX_EVENTS: i32 = 256;
const BUFFER_SIZE: usize = 256;

#[derive(StructOpt, Debug)]
#[structopt(name = "epollserver")]
struct Opt {
    #[structopt(short, long, default_value = "9090")]
    port: u16
}

struct ClientState {
    stream: TcpStream,
    buf: Vec<u8>
}

impl ClientState {
    pub fn with_stream(stream: TcpStream) -> ClientState {
        ClientState {
            stream,
            buf: Vec::with_capacity(BUFFER_SIZE),
        }
    }
}

fn broadcast_message(orator: &ClientState, clients: &mut HashMap<i32, ClientState>){
    let stream = orator.stream.borrow();
    let fd = stream.as_raw_fd();
    let message = orator.buf.as_ref();

    for (cfd, client) in clients.iter_mut() {
        if *cfd != fd {
            if let Err(e) = client.stream.write(message) {
                eprintln!("write error (fd = {}): {e}", *cfd);
            }
        }
    }
}

fn handle_client(epfd: i32, cfd: i32, clients: &mut HashMap<i32, ClientState>) {
    let client = clients.get_mut(&cfd).unwrap();
    let stream = client.stream.borrow_mut();
    let cfd = stream.as_raw_fd();

    match stream.read(&mut client.buf) {
        Ok(bytes) => {
            if bytes == 0 {
                remove_client(epfd, cfd, clients);
                return;
            }

            if client.buf.ends_with(b"\n") || client.buf.len() == BUFFER_SIZE {
                broadcast_message(client, clients);
                client.buf.clear();
            } 
        },
        Err(e) => {
            match e.kind() {
                ErrorKind::WouldBlock => return,
                _ =>  { 
                    eprintln!("read error (removing fd = {}): {}", cfd, Error::last_os_error());
                    remove_client(epfd, cfd, clients);
                }
            }
        }
    }
}

fn remove_client(epfd: i32, cfd: i32, clients: &mut HashMap<i32, ClientState>) {
    unsafe { libc::epoll_ctl(epfd, libc::EPOLL_CTL_DEL, cfd, std::ptr::null_mut()); }
    clients.remove(&cfd);
    println!("removed client {}", cfd);
}

fn accept_client(epfd: i32, listener: &TcpListener, clients: &mut HashMap<i32, ClientState>) -> Result<()> {
    let (stream, _) = match listener.accept() {
        Ok(s) => s,
        Err(e) => return Err(e)
    };

    if let Err(e) = stream.set_nonblocking(true) {
        return Err(e);
    }
    let cfd = stream.as_raw_fd();
    println!("accepted a client (fd = {})", cfd);

    let mut e = libc::epoll_event {
        events: libc::EPOLLIN as u32,
        u64: cfd as u64
    };

    let ret = unsafe { libc::epoll_ctl(epfd, libc::EPOLL_CTL_ADD, cfd, &mut e) };
    if ret < 0 {
        eprintln!("failed to add client to epoll");
        stream.shutdown(Shutdown::Both);
        return Err(Error::last_os_error());
    }

    let client = ClientState::with_stream(stream);
    clients.insert(cfd, client);

    Ok(())
}

fn await_clients(listener: TcpListener, epfd: i32, events: *mut libc::epoll_event) {
    let mut clients: HashMap<i32, ClientState> = HashMap::new();

    loop {
        let ready = unsafe { libc::epoll_wait(epfd, events, MAX_EVENTS, -1) };
        if ready < 0 {
            eprintln!("epoll_wait error: {}", Error::last_os_error());
            continue;
        }

        for i in 0..ready as isize {
            if let Some(event) = unsafe { events.offset(i).as_ref() } {
                if event.u64 == listener.as_raw_fd() as u64 {
                    if let Err(e) = accept_client(epfd, &listener, &mut clients) {
                        eprintln!("{e}");
                    }
                } else {
                    handle_client(epfd, event.u64 as i32, &mut clients);
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
    let events: *mut libc::epoll_event = Vec::with_capacity(MAX_EVENTS as usize).as_mut_ptr();
    await_clients(listener, epfd, events);

    Ok(())
}
