use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub struct TestServer {
    pub endpoint: String,
    request: Arc<Mutex<String>>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl TestServer {
    pub fn request(&self) -> String {
        self.request.lock().unwrap().clone()
    }

    pub fn join(&self) {
        if let Some(handle) = self.handle.lock().unwrap().take() {
            handle.join().unwrap();
        }
    }
}

pub fn spawn_reranker(status: &'static str, body: &'static str) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let request = Arc::new(Mutex::new(String::new()));
    let request_for_thread = Arc::clone(&request);
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(250)))
            .unwrap();
        let mut bytes = Vec::new();
        loop {
            let mut chunk = [0_u8; 4096];
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => {
                    bytes.extend_from_slice(&chunk[..read]);
                    if http_request_complete(&bytes) {
                        break;
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    break;
                }
                Err(error) => panic!("read reranker request: {error}"),
            }
        }
        *request_for_thread.lock().unwrap() = String::from_utf8(bytes).unwrap();
        let response = format!(
            "{status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
    });
    TestServer {
        endpoint,
        request,
        handle: Mutex::new(Some(handle)),
    }
}

fn http_request_complete(bytes: &[u8]) -> bool {
    let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let content_len = headers
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length: "))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    bytes.len() >= header_end + 4 + content_len
}
