use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    time::Duration,
};
use straja_sql::{mail, model::MailSettings};
#[test]
fn smtp_refuses_server_without_starttls() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream.write_all(b"220 localhost test SMTP\r\n").unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(line.starts_with("EHLO"));
        stream
            .write_all(b"250-localhost\r\n250 SIZE 10000\r\n")
            .unwrap();
        line.clear();
        let _ = reader.read_line(&mut line);
        assert!(!line.starts_with("MAIL") && !line.starts_with("AUTH"));
    });
    let m = MailSettings {
        host: "localhost".into(),
        port,
        username: String::new(),
        from: "test@example.invalid".into(),
        to: "test@example.invalid".into(),
        implicit_tls: false,
    };
    assert!(mail::send(&m, "", "Test", "Local test only").is_err());
    server.join().unwrap();
}
