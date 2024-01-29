use std::net::TcpListener;
use std::io::Result;
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
#[structopt(name = "epollserver")]
struct Opt {
    #[structopt(short, long, default_value = "9090")]
    port: u16
}

fn main() -> Result<()> {
    println!("epoll server initializing...");

    let opt = Opt::from_args();
    let addr = format!("localhost:{}", opt.port);

    let _listener = TcpListener::bind(addr)?;

    Ok(())
}
