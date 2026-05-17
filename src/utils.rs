use std::env;
use std::error::Error;
use std::io::{Read, Write};
use std::process::Command;

pub fn print_and_flush(data: &str) {
    print!("{}", data);
    std::io::stdout().flush().unwrap();
}

pub fn run_shell_command(cmd: &str) -> String {
    let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    let output = Command::new(shell).arg("-c").arg(cmd).output().unwrap();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return format!("command failed: {}", stderr);
    }
    String::from_utf8_lossy(&output.stdout).to_string()
}

/* This struct is used for streaming responses - it reads
 * from a reader and returns the entire output split by a given
 * separator (newline) */
pub struct SplitStream<Reader, const N: usize = 256> {
    buf: [u8; N],
    bufcount: usize,
    separator: u8,
    string: Vec<u8>,
    reader: Reader,
}

impl<Reader, const N: usize> SplitStream<Reader, N> {
    pub fn new(reader: Reader, separator: u8) -> Self {
        Self {
            buf: [0; _],
            bufcount: 0,
            separator,
            string: vec![],
            reader,
        }
    }

    pub fn next(&mut self) -> Result<Vec<u8>, Box<dyn Error>>
    where
        Reader: Read,
    {
        loop {
            let n_read = self.reader.read(&mut self.buf[self.bufcount..])?;
            self.bufcount += n_read;

            if let Some(pos) = self.buf[0..self.bufcount]
                .iter()
                .position(|&b| b == self.separator)
            {
                self.string.extend_from_slice(&self.buf[0..pos]);
                self.buf.rotate_left(pos + 1);
                self.bufcount -= pos + 1;
                return Ok(std::mem::take(&mut self.string));
            } else {
                self.string.extend_from_slice(&self.buf[0..self.bufcount]);
                self.bufcount = 0;
            }
            if n_read == 0 {
                return Ok(std::mem::take(&mut self.string));
            }
        }
    }
}
