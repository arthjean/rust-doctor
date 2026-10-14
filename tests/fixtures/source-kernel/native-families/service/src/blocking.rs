use std::fs;
use std::thread::sleep;
use std::time::Duration;

pub async fn sleeps() {
    std::thread::sleep(Duration::from_millis(1));
}

pub async fn sleeps_imported() {
    sleep(Duration::from_millis(1));
}

pub async fn reads() -> std::io::Result<Vec<u8>> {
    fs::read("Cargo.toml")
}

pub async fn reads_text() -> std::io::Result<String> {
    std::fs::read_to_string("Cargo.toml")
}

pub async fn writes() -> std::io::Result<()> {
    std::fs::write("out.txt", b"x")
}

pub async fn copies() -> std::io::Result<u64> {
    std::fs::copy("a.txt", "b.txt")
}

pub async fn removes() -> std::io::Result<()> {
    std::fs::remove_file("a.txt")
}

pub async fn creates_directories() -> std::io::Result<()> {
    std::fs::create_dir_all("out")
}

pub async fn opens() -> std::io::Result<std::fs::File> {
    std::fs::File::open("Cargo.toml")
}

pub async fn creates() -> std::io::Result<std::fs::File> {
    fs::File::create("out.txt")
}

pub async fn connects() -> std::io::Result<std::net::TcpStream> {
    std::net::TcpStream::connect("127.0.0.1:1")
}

pub async fn waits_for_output() -> std::io::Result<std::process::Output> {
    std::process::Command::new("true").output()
}

pub async fn waits_for_status_through_a_binding() -> std::io::Result<std::process::ExitStatus> {
    let mut child = std::process::Command::new("true");
    child.arg("--version").status()
}

pub fn in_an_async_block() -> impl std::future::Future<Output = ()> {
    async {
        std::thread::sleep(Duration::from_millis(1));
    }
}

pub async fn in_a_closure(paths: &[&str]) -> Vec<std::io::Result<Vec<u8>>> {
    paths.iter().map(|path| std::fs::read(path)).collect()
}

pub async fn blocking_http() -> Result<(), ()> {
    reqwest::blocking::get("http://localhost")
}

#[cfg(test)]
mod tests {
    #[test]
    fn an_inline_test_module() {
        let _task = async {
            std::thread::sleep(std::time::Duration::from_millis(1));
        };
    }
}
