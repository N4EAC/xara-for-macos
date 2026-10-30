use crate::command::{Command, parse};
use crate::writer::Writer;
use std::io::{self, Read};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

pub const MAX_LINE: usize = 1024;
pub const DATA_CHUNK: usize = 65536;
pub const INPUT_QUEUE: usize = 64;
pub(crate) const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

#[derive(Debug)]
pub enum Input {
    Command(Command),
    Wrong(String),
    Data(Vec<u8>),
    Connected,
    Closed,
}

#[derive(Clone, Copy)]
enum Port {
    Cmd,
    Data,
}

#[derive(Default)]
struct Clients {
    cmd: Option<Writer>,
    data: Option<Writer>,
}

impl Clients {
    fn slot(&mut self, p: Port) -> &mut Option<Writer> {
        match p {
            Port::Cmd => &mut self.cmd,
            Port::Data => &mut self.data,
        }
    }
}

type Shared = Arc<Mutex<Clients>>;

#[derive(Default)]
struct Hold {
    on: Mutex<bool>,
    cv: Condvar,
}

impl Hold {
    fn set(&self, on: bool) {
        *self.on.lock().unwrap() = on;
        self.cv.notify_all();
    }

    fn wait(&self) {
        drop(self.cv.wait_while(self.on.lock().unwrap(), |on| *on).unwrap());
    }
}

pub struct Host {
    pub input: Receiver<Input>,
    clients: Shared,
    hold: Arc<Hold>,
    stop: Arc<AtomicBool>,
    workers: Vec<thread::JoinHandle<()>>,
    pub cmd_addr: SocketAddr,
    pub data_addr: SocketAddr,
}

impl Host {
    /// In-process operation without opening command or data sockets.
    pub fn disabled() -> Self {
        let (_, input) = sync_channel(INPUT_QUEUE);
        Self {
            input,
            clients: Shared::default(),
            hold: Arc::default(),
            stop: Arc::default(),
            workers: Vec::new(),
            cmd_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            data_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
        }
    }

    pub fn bind(addr: &str, port: u16) -> io::Result<Self> {
        let cl = bind_fixed(addr, port)?;
        let data_port = match port {
            0 => 0,
            p => p.checked_add(1).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "port must be < 65535"))?,
        };
        let dl = bind_fixed(addr, data_port)?;
        let (tx, input) = sync_channel(INPUT_QUEUE);
        let clients = Shared::default();
        let hold = Arc::<Hold>::default();
        let mut h = Self {
            input,
            clients: clients.clone(),
            hold: hold.clone(),
            stop: Arc::default(),
            workers: Vec::new(),
            cmd_addr: cl.local_addr()?,
            data_addr: dl.local_addr()?,
        };
        h.workers.push(serve(cl, Port::Cmd, clients.clone(), tx.clone(), hold.clone(), h.stop.clone())?);
        h.workers.push(serve(dl, Port::Data, clients, tx, hold, h.stop.clone())?);
        Ok(h)
    }

    /// Stop listener/client threads before restarting an embedded modem.
    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.hold.set(false);
        {
            let mut clients = self.clients.lock().unwrap();
            for w in [clients.cmd.take(), clients.data.take()].into_iter().flatten() {
                w.close();
            }
        }
        while self.input.try_recv().is_ok() {}
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }

    pub fn hold_data(&self, on: bool) {
        self.hold.set(on);
    }

    pub fn line(&self, s: &str) {
        self.send(Port::Cmd, format!("{s}\r").as_bytes());
    }

    pub fn data(&self, b: &[u8]) {
        self.send(Port::Data, b);
    }

    fn send(&self, p: Port, b: &[u8]) {
        if let Some(w) = self.clients.lock().unwrap().slot(p) {
            w.send(b);
        }
    }

    pub fn has_cmd_client(&self) -> bool {
        self.clients.lock().unwrap().cmd.is_some()
    }

    pub fn has_data_client(&self) -> bool {
        self.clients.lock().unwrap().data.is_some()
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.hold.set(false);
    }
}

#[cfg(not(target_os = "macos"))]
fn set_reuse(l: &TcpListener, on: bool) -> io::Result<()> {
    let v: libc::c_int = on.into();
    let len = std::mem::size_of_val(&v) as libc::socklen_t;
    let r =
        unsafe { libc::setsockopt(l.as_raw_fd(), libc::SOL_SOCKET, libc::SO_REUSEADDR, (&raw const v).cast(), len) };
    if r != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn pause(l: &TcpListener) -> io::Result<()> {
    if unsafe { libc::shutdown(l.as_raw_fd(), libc::SHUT_RD) } != 0 {
        return Err(io::Error::last_os_error());
    }
    set_reuse(l, false)
}

#[cfg(not(target_os = "macos"))]
fn resume(l: &TcpListener) -> io::Result<()> {
    set_reuse(l, true)?;
    if unsafe { libc::listen(l.as_raw_fd(), 128) } != 0 {
        let e = io::Error::last_os_error();
        let _ = set_reuse(l, false);
        return Err(e);
    }
    Ok(())
}

fn bind_fixed(addr: &str, port: u16) -> io::Result<TcpListener> {
    if port != 0 {
        return TcpListener::bind((addr, port));
    }
    for _ in 0..16 {
        let p = TcpListener::bind((addr, 0))?.local_addr()?.port();
        if let Ok(l) = TcpListener::bind((addr, p)) {
            return Ok(l);
        }
    }
    Err(io::Error::new(io::ErrorKind::AddrInUse, "no free port"))
}

#[cfg(not(target_os = "macos"))]
fn serve(
    l: TcpListener,
    port: Port,
    clients: Shared,
    tx: SyncSender<Input>,
    hold: Arc<Hold>,
    stop: Arc<AtomicBool>,
) -> io::Result<thread::JoinHandle<()>> {
    l.set_nonblocking(true)?;
    let name = match port {
        Port::Cmd => "host-cmd-accept",
        Port::Data => "host-data-accept",
    };
    thread::Builder::new().name(name.into()).spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            let s = match l.accept() {
                Ok((s, _)) => s,
                Err(_) => {
                    thread::sleep(ACCEPT_BACKOFF);
                    continue;
                }
            };
            if stop.load(Ordering::SeqCst) {
                break;
            }
            let _ = s.set_nonblocking(false);
            if let Err(e) = pause(&l) {
                eprintln!("xara: {name} cannot refuse a second client: {e}");
            }
            if attach(s, port, &clients, &tx, &hold, &stop).is_err() {
                return;
            }
            if let Err(e) = resume(&l) {
                eprintln!("xara: {name} cannot listen again, retrying: {e}");
                while !stop.load(Ordering::SeqCst) && resume(&l).is_err() {
                    thread::sleep(ACCEPT_BACKOFF);
                }
            }
        }
    })
}

// Darwin cannot shutdown a listening socket while keeping its port reserved.
// Keep accepting while its single reader runs, and immediately reset extra clients.
#[cfg(target_os = "macos")]
fn serve(
    l: TcpListener,
    port: Port,
    clients: Shared,
    tx: SyncSender<Input>,
    hold: Arc<Hold>,
    stop: Arc<AtomicBool>,
) -> io::Result<thread::JoinHandle<()>> {
    l.set_nonblocking(true)?;
    thread::Builder::new().name("host-accept".into()).spawn(move || {
        let mut reader: Option<thread::JoinHandle<()>> = None;
        while !stop.load(Ordering::SeqCst) {
            if reader.as_ref().is_some_and(|r| r.is_finished()) {
                if let Some(reader) = reader.take() {
                    let _ = reader.join();
                }
            }
            let socket = match l.accept() {
                Ok((socket, _)) => socket,
                Err(_) => {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
            };
            if stop.load(Ordering::SeqCst) {
                break;
            }
            if reader.is_some() {
                let linger = libc::linger { l_onoff: 1, l_linger: 0 };
                // SAFETY: socket is valid and linger has the size passed to setsockopt.
                unsafe {
                    libc::setsockopt(
                        socket.as_raw_fd(),
                        libc::SOL_SOCKET,
                        libc::SO_LINGER,
                        (&raw const linger).cast(),
                        std::mem::size_of_val(&linger) as libc::socklen_t,
                    );
                }
                continue;
            }
            let _ = socket.set_nonblocking(false);
            let (clients, tx, hold, stopping) = (clients.clone(), tx.clone(), hold.clone(), stop.clone());
            reader = thread::Builder::new()
                .name("host-client".into())
                .spawn(move || {
                    let _ = attach(socket, port, &clients, &tx, &hold, &stopping);
                })
                .ok();
        }
        if let Some(reader) = reader {
            let _ = reader.join();
        }
    })
}

fn attach(
    s: TcpStream,
    port: Port,
    clients: &Shared,
    tx: &SyncSender<Input>,
    hold: &Hold,
    stop: &AtomicBool,
) -> Result<(), ()> {
    let _ = s.set_nodelay(true);
    let Ok(w) = Writer::spawn(&s) else { return Ok(()) };
    {
        let mut clients = clients.lock().unwrap();
        if stop.load(Ordering::SeqCst) {
            w.close();
            return Ok(());
        }
        *clients.slot(port) = Some(w);
    }
    tx.send(Input::Connected).map_err(drop)?;
    match port {
        Port::Cmd => read_commands(s, tx),
        Port::Data => read_data(s, tx, hold),
    }
    let mut c = clients.lock().unwrap();
    if c.slot(port).is_some() {
        for w in [c.cmd.take(), c.data.take()].into_iter().flatten() {
            w.close();
        }
        drop(c);
        tx.send(Input::Closed).map_err(drop)?;
    }
    Ok(())
}

fn read_commands(mut s: TcpStream, tx: &SyncSender<Input>) {
    let mut buf = [0u8; 4096];
    let mut line = Vec::new();
    let mut long = false;
    while let Ok(n @ 1..) = s.read(&mut buf) {
        for &b in &buf[..n] {
            match b {
                b'\r' => {
                    let l = String::from_utf8_lossy(&line).into_owned();
                    line.clear();
                    if std::mem::take(&mut long) {
                        continue;
                    }
                    let msg = parse(&l).map_or(Input::Wrong(l), Input::Command);
                    if tx.send(msg).is_err() {
                        return;
                    }
                }
                b'\n' => {}
                _ if line.len() >= MAX_LINE => {
                    line.clear();
                    long = true;
                }
                _ if long => {}
                _ => line.push(b),
            }
        }
    }
}

fn read_data(mut s: TcpStream, tx: &SyncSender<Input>, hold: &Hold) {
    let mut buf = vec![0; DATA_CHUNK];
    loop {
        hold.wait();
        let Ok(n @ 1..) = s.read(&mut buf) else { return };
        if tx.send(Input::Data(buf[..n].to_vec())).is_err() {
            return;
        }
    }
}
