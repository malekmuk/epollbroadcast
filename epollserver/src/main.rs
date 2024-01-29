use std::net::TcpListener;
use std::io::{Error, ErrorKind, Result};
use std::os::fd::{AsRawFd, RawFd};
use structopt::StructOpt;

const MAX_EVENTS: libc::c_int = 1024;

#[derive(StructOpt, Debug)]
#[structopt(name = "epollserver")]
struct Opt {
    #[structopt(short, long, default_value = "9090")]
    port: u16
}

fn handle_client(cfd: i32, epfd: i32) {
    unsafe { 
        libc::epoll_ctl(epfd, libc::EPOLL_CTL_DEL, cfd, std::ptr::null_mut()); 
        libc::close(cfd);
    }
}

fn accept_client(sockfd: i32, epfd: i32) {
    let cfd = unsafe { 
        libc::accept4(
            sockfd, 
            std::ptr::null_mut(), 
            std::ptr::null_mut(), 
            libc::SOCK_NONBLOCK
        )
    };
    println!("Accepted a client! (fd = {})", cfd);

    let mut e = libc::epoll_event {
        events: libc::EPOLLIN as u32,
        u64: cfd as u64
    };

    unsafe { libc::epoll_ctl(epfd, libc::EPOLL_CTL_ADD, cfd, &mut e); }
}

fn await_clients(sockfd: i32, epfd: i32, events: *mut libc::epoll_event) {
    loop {
        println!("Waiting for events!");

        let ready = unsafe { libc::epoll_wait(epfd, events, MAX_EVENTS, -1) };
        println!("Got {} events!", ready);

        for i in 0..ready as isize {
            if let Some(event) = unsafe { events.offset(i).as_mut() } {
                if event.u64 == sockfd as u64 {
                    accept_client(sockfd, epfd);
                } else {
                    handle_client(event.u64 as i32, epfd);
                }
            }
        }
    }
}

fn epoll_init(sockfd: RawFd) -> Result<i32> {
    unsafe {
        let epfd = libc::epoll_create1(0);

        if epfd >= 0 {
            let mut e = libc::epoll_event {
                events: libc::EPOLLIN as u32,
                u64: sockfd as u64
            };
            
            if libc::epoll_ctl(epfd, libc::EPOLL_CTL_ADD, sockfd, &mut e) == 0 {
                return Ok(epfd);
            }
            else {
                let errmsg = format!("epoll ctl failed to add server fd {} -- OS Error: {}", sockfd, *libc::__errno_location());
                return Err(Error::new(ErrorKind::Other, errmsg));
            }
        }

        let errmsg = format!("epoll ctl failed to add server fd -- OS Error: {}", *libc::__errno_location());
        Err(Error::new(ErrorKind::Other, errmsg))
    }
}

fn main() -> Result<()> {
    let opt = Opt::from_args();
    let addr = format!("localhost:{}", opt.port);
    println!("epoll server attempting to listen on port {}...\n", opt.port);

    let listener = TcpListener::bind(addr)?;
    let sockfd = listener.as_raw_fd();

    let epfd = epoll_init(sockfd)?;
    let events: *mut libc::epoll_event = Vec::with_capacity(MAX_EVENTS as usize).as_mut_ptr();
    await_clients(sockfd, epfd, events);

    Ok(())
}
