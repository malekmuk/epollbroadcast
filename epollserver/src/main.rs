use std::net::TcpListener;
use std::io::Result;
use std::os::fd::{AsRawFd, RawFd};
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
#[structopt(name = "epollserver")]
struct Opt {
    #[structopt(short, long, default_value = "9090")]
    port: u16
}

const MAX_EVENTS: libc::c_int = 1024;

fn epoll_init(sockfd: RawFd) -> Option<i32> {
    unsafe {
        let epfd = libc::epoll_create1(0);

        if epfd >= 0 {
            let mut e = libc::epoll_event {
                events: libc::EPOLLIN as u32,
                u64: sockfd as u64
            };
            
            if libc::epoll_ctl(epfd, libc::EPOLL_CTL_ADD, sockfd, &mut e) == 0 {
                return Some(epfd);
            }
        }
    }

    None
}

fn main() -> Result<()> {
    let opt = Opt::from_args();
    let addr = format!("localhost:{}", opt.port);
    println!("epoll server attempting to listen on port {}...\n", opt.port);

    let listener = TcpListener::bind(addr)?;

    if let Some(epfd) = epoll_init(listener.as_raw_fd()) {
        let events: *mut libc::epoll_event = Vec::with_capacity(MAX_EVENTS as usize).into_boxed_slice().as_mut_ptr();
        
        loop {
            unsafe {
                println!("Waiting for events!");
                let ready = libc::epoll_wait(epfd, events, MAX_EVENTS, -1);

                for i in 0..ready {
                    println!("Got {} events!\n", ready);
                    if (*(events.offset(i as isize))).u64 == epfd as u64 {

                    }
                }
            }
        }
    }

    Ok(())
}
