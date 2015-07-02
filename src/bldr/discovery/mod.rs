pub mod etcd;

use std::collections::HashMap;
use std::sync::mpsc::{channel, Sender, Receiver, TryRecvError};
use std::thread;
use std::fmt::{self, Debug};
use ansi_term::Colour::{White};

use pkg::Package;
use error::{BldrResult, BldrError};

// 1. Watch a key for changes with a reconnect timer
// 2. Write values to a key
// 3. Write values to a key with a TTL, and keep it alive
// 4. Check for the absence of a key
//
// worker.discovery.watch("/foo/bar/baz", "101_leader", reconnect);
// worker.discovery.write("/foo/bar/baz", "101_leader", toml_string, Some(ttl));
// # Returns the last value we got for the watch location - none indicates the key is absent
// worker.discovery.status("/foo/bar/baz") -> Some(data)

#[derive(Debug)]
pub struct Discovery {
    watchers: Vec<DiscoveryWatcher>,
    writers: Vec<DiscoveryWriter>,
    status: HashMap<String, DiscoveryResponse>,
    backend: Backend
}

#[derive(Debug, Clone, Copy)]
pub enum Backend {
    Etcd
}

impl Discovery {
    pub fn new(backend: Backend) -> Discovery {
        Discovery{
            watchers: Vec::new(),
            writers: Vec::new(),
            status: HashMap::new(),
            backend: backend
        }
    }

    pub fn watch(&mut self, mut dw: DiscoveryWatcher) {
        dw.backend(self.backend);
        dw.start();
        self.watchers.push(dw);
    }

    pub fn status(&self, key: &str) -> Option<&DiscoveryResponse> {
        self.status.get(&String::from(key))
    }

    pub fn next(&mut self) -> BldrResult<()> {
        // Writers should come first, then watchers - but baby steps
        for watch in self.watchers.iter_mut() {
            let result = try!(watch.try_recv());
            if let Some(msg) = result {
                println!("I received a {:?}", msg);
                self.status.insert(msg.key.clone(), msg);
            }
        }
        Ok(())
    }

    pub fn stop(&mut self) -> BldrResult<()> {
        for watch in self.watchers.iter_mut() {
            watch.stop();
        }
        Ok(())
    }
}

pub struct DiscoveryWatcher {
    package: Package,
    key: String,
    filename: String,
    wait: bool,
    reconnect_interval: u32,
    backend: Option<Backend>,
    rx: Option<Receiver<Option<String>>>,
    tx: Option<Sender<bool>>,
}

impl Debug for DiscoveryWatcher {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f,
               "DiscoveryWatcher{{key: {}, filename: {}, reconnect_interval: {}, backend: {:?}, rx: {}}}",
               self.key, self.filename, self.reconnect_interval, self.backend, self.rx.is_some())
    }
}

impl DiscoveryWatcher {
    pub fn new(package: Package, key: String, filename: String, reconnect_interval: u32, wait: bool) -> DiscoveryWatcher {
        DiscoveryWatcher{
            package: package,
            key: key,
            filename: filename,
            wait: wait,
            reconnect_interval: reconnect_interval,
            backend: None,
            tx: None,
            rx: None,
        }
    }

    fn backend(&mut self, backend: Backend) {
        self.backend = Some(backend)
    }

    fn start(&mut self) {
        let preamble = format!("{}({})", self.package.name, White.bold().paint("D"));
        println!("   {}: Watching {}", preamble, self.key);
        let (b_tx, b_rx) = channel();
        let (w_tx, w_rx) = channel();
        self.tx = Some(w_tx);
        self.rx = Some(b_rx);
        match self.backend {
            ref Etcd => etcd::watch(&self.key, self.reconnect_interval, self.wait, b_tx, w_rx)
        }
    }

    fn stop(&mut self) {
        let tx = self.tx.as_ref().unwrap();
        tx.send(true).unwrap();
    }

    fn try_recv(&mut self) -> BldrResult<Option<DiscoveryResponse>> {
        let rx = self.rx.as_ref().unwrap();
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return Ok(None),
            Err(e) => return Err(BldrError::from(e))
        };
        match result {
            Some(s) => {
                try!(self.package.write_toml_string(&self.filename, &s));
                Ok(Some(DiscoveryResponse{key: self.key.clone(), value: Some(String::from(s))}))
            },
            None => Ok(Some(DiscoveryResponse{key: self.key.clone(), value: None}))
        }
    }
}

struct DiscoveryWriter {
    package: Package,
    key: String,
    value: Option<String>,
    ttl: Option<u32>,
    backend: Option<Backend>,
    rx: Option<Receiver<Option<String>>>,
    tx: Option<Sender<bool>>,
}

impl Debug for DiscoveryWriter {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f,
               "DiscoveryWriter{{package: {:?}, key: {}, value: {:?}, ttl: {:?}, backend: {:?}, rx: {}, tx: {}}}",
               self.package, self.key, self.value, self.ttl, self.backend, self.rx.is_some(), self.tx.is_some())
    }
}

impl DiscoveryWriter {
    pub fn new(package: Package, key: String, value: Option<String>, ttl: Option<u32>) -> DiscoveryWriter {
        DiscoveryWriter{
            package: package,
            key: key,
            value: value,
            ttl: ttl,
            backend: None,
            tx: None,
            rx: None
        }
    }

    fn backend(&mut self, backend: Backend) {
        self.backend = Some(backend)
    }
}

#[derive(Debug)]
pub struct DiscoveryResponse {
    pub key: String,
    pub value: Option<String>,
}

#[derive(Debug)]
pub struct DiscoveryWriteResponse {
    pub key: String,
    pub value: Option<String>,
}
