use serde_json::Value;
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: BTreeMap<String, String>,
    pub body: Value,
}

pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
    pub length: Option<usize>,
    pub location: Option<String>,
}

impl Reply {
    pub fn json(value: Value) -> Self {
        Self::bytes(serde_json::to_vec(&value).unwrap())
    }
    pub fn bytes(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            body: body.into(),
            length: None,
            location: None,
        }
    }
}

pub struct Server {
    pub url: String,
    stopped: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    pub fn new(mut handler: impl FnMut(Request) -> Reply + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&stopped);
        let thread = std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => serve(stream, &mut handler),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("mock listener: {error}"),
                }
            }
        });
        Self {
            url,
            stopped,
            thread: Some(thread),
        }
    }
}

fn serve(mut stream: TcpStream, handler: &mut impl FnMut(Request) -> Reply) {
    // Windows accept inherits the listener's nonblocking mode. The parser below
    // is deliberately blocking, otherwise an early WouldBlock drops valid clients.
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut first = String::new();
    if reader.read_line(&mut first).is_err() || first.is_empty() {
        return;
    }
    let fields: Vec<_> = first.split_whitespace().collect();
    let method = fields[0].to_owned();
    let path = fields[1].to_owned();
    let mut headers = BTreeMap::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line.trim().is_empty() {
            break;
        }
        let (name, value) = line.split_once(':').unwrap();
        headers.insert(name.to_lowercase(), value.trim().to_owned());
    }
    let length = headers
        .get("content-length")
        .map(|v| v.parse().unwrap())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    let reply = handler(Request {
        method,
        path,
        headers,
        body: if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body).unwrap()
        },
    });
    let mut head = format!(
        "HTTP/1.1 {} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        reply.status,
        reply.length.unwrap_or(reply.body.len())
    );
    if let Some(location) = reply.location {
        head.push_str(&format!("Location: {location}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream
        .write_all(head.as_bytes())
        .and_then(|_| stream.write_all(&reply.body));
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
            && !std::thread::panicking()
        {
            panic!("mock server failed");
        }
    }
}
