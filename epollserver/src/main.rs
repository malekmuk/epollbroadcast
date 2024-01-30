use std::collections::HashSet;
use std::net::TcpListener;
use std::io::{Error, Result};
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
    fd: i32,
    buf: Vec<u8>
}

impl ClientState {
    pub fn with_fd(fd: i32) -> ClientState {
        ClientState {
            fd,
            buf: Vec::with_capacity(BUFFER_SIZE),
        }
    }
}

fn broadcast_message(orator: &ClientState, active: &HashSet<i32>) {
    let message = orator.buf.as_ptr() as *const libc::c_void;
    let len = orator.buf.len();

    for client in active.iter() {
        if *client != orator.fd {
            unsafe { libc::write(*client, message, len) };
        }
    }
}

fn handle_client(epfd: i32, cfd: i32, clients: &mut Vec<ClientState>, active: &mut HashSet<i32>) {
    let client = clients.get_mut(cfd as usize).unwrap();
    let bufptr = unsafe { client.buf.as_mut_ptr().offset(client.buf.len() as isize) as *mut libc::c_void };
    let count = client.buf.capacity() - client.buf.len();
    let ret = unsafe { libc::read(cfd, bufptr, count) };

    match ret {
        -1 => {
            unsafe {
                let errno = *(libc::__errno_location());
                match errno {
                    libc::EAGAIN => { return; },
                    _ => {
                        eprintln!("read error (removing fd = {}): {}", cfd, Error::last_os_error());
                        remove_client(epfd, cfd, active);
                    }
                }
            }
        }
        0 => {
            remove_client(epfd, cfd, active);
        }
        _ => {
            unsafe { client.buf.set_len(client.buf.len() + ret as usize) };

            if client.buf.ends_with(b"\n") || client.buf.len() == BUFFER_SIZE {
                broadcast_message(client, active);
                client.buf.clear();
            } 
        }
    }
}

fn remove_client(epfd: i32, cfd: i32, active: &mut HashSet<i32>) {
    unsafe { 
        libc::epoll_ctl(epfd, libc::EPOLL_CTL_DEL, cfd, std::ptr::null_mut()); 
        libc::close(cfd);
    }

    active.remove(&cfd);
    println!("removed client {}", cfd);
}

fn accept_client(epfd: i32, sockfd: i32, clients: &mut Vec<ClientState>) -> Result<i32> {
    let cfd = unsafe { 
        libc::accept4(
            sockfd, 
            std::ptr::null_mut(), 
            std::ptr::null_mut(), 
            libc::SOCK_NONBLOCK
        )
    };
    if cfd < 0 {
        eprintln!("failed to accept client (fd = {})", cfd);
        return Err(Error::last_os_error());
    }

    println!("accepted a client (fd = {})", cfd);

    let mut e = libc::epoll_event {
        events: libc::EPOLLIN as u32,
        u64: cfd as u64
    };

    let ret = unsafe { libc::epoll_ctl(epfd, libc::EPOLL_CTL_ADD, cfd, &mut e) };
    if ret < 0 {
        eprintln!("failed to add client to epoll");
        unsafe { libc::close(cfd) };
        return Err(Error::last_os_error());
    }

    let client = clients.get_mut(cfd as usize).unwrap();
    client.fd = cfd;
    client.buf.clear();

    Ok(cfd)
}

fn await_clients(sockfd: i32, epfd: i32, events: *mut libc::epoll_event) {
    let mut clients: Vec<ClientState> = Vec::with_capacity(MAX_EVENTS as usize);
    let mut active: HashSet<i32> = HashSet::new();
    for _i in 0..MAX_EVENTS {
        clients.push(ClientState::with_fd(-1));
    }

    loop {
        let ready = unsafe { libc::epoll_wait(epfd, events, MAX_EVENTS, -1) };
        if ready < 0 {
            eprintln!("epoll_wait error: {}", Error::last_os_error());
            continue;
        }

        for i in 0..ready as isize {
            if let Some(event) = unsafe { events.offset(i).as_ref() } {
                if event.u64 == sockfd as u64 {
                    if let Ok(cfd) = accept_client(epfd, sockfd, &mut clients) {
                        active.insert(cfd);
                    }
                } else {
                    handle_client(epfd, event.u64 as i32, &mut clients, &mut active);
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

    let sockfd = listener.as_raw_fd();
    let epfd = epoll_init(sockfd)?;
    let events: *mut libc::epoll_event = Vec::with_capacity(MAX_EVENTS as usize).as_mut_ptr();
    await_clients(sockfd, epfd, events);

    Ok(())
}
