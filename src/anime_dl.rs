use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, TcpStream};
use std::str::from_utf8;
use std::{thread, time, fs};
use std::path::PathBuf;
use rand::Rng;
use anyhow::{Result, anyhow};
use once_cell::sync::Lazy;
use std::sync::mpsc::Sender;
use regex::Regex;
use std::thread::sleep;

static DCC_SEND_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"DCC SEND "?(.*)"? (\d+) (\d+) (\d+)"#).expect("Failed to create regex.")
});
static MODE_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"MODE .* :\+.*"#).expect("Failed to create regex.")
});

pub struct IRCRequest {
    pub server: String,
    pub channel: String,
    pub nickname: String,
    pub bot: Vec<String>,
    pub packages: Vec<String>,
}

#[derive(Clone)]
struct DCCSend {
    filename: String,
    ip: IpAddr,
    port: String,
    file_size: usize,
}

struct IRCConnection {
    socket: TcpStream,
    partial_msg: String,
}

impl IRCConnection {
    fn read_message(&mut self) -> Result<Option<String>> {
        let mut buffer = [0; 4];
        let count = self.socket.read(&mut buffer[..])?;
        self.partial_msg.push_str(from_utf8(&buffer[..count]).unwrap_or_default());
        //println!("{}", self.message_builder);
        if self.partial_msg.contains('\n') {
            let endline_offset = self.partial_msg.find('\n').ok_or_else(|| anyhow!("NoneError"))? + 1;
            let message = self.partial_msg.get(..endline_offset).ok_or_else(|| anyhow!("NoneError"))?.to_string();
            self.partial_msg.replace_range(..endline_offset, "");
            Ok(Some(message))
        } else {
            Ok(None)
        }
    }
}

pub fn connect_and_download(request: IRCRequest, channel_senders: Vec<Sender<i64>>, status_bar_sender: Sender<String>, dir_path: PathBuf) -> Result<()> {
    status_bar_sender.send(format!("Connecting to Rizon..."))?;

    let mut download_handles = Vec::new();
    let mut has_joined = false;
    let stream = log_in(&request)?;
    let mut connection : IRCConnection = IRCConnection { socket: stream, partial_msg: "".to_string()};

    let mut next = time::Instant::now() + time::Duration::from_millis(500);
    let timeout_threshold = 5;
    let mut timeout_counter = 0;
    status_bar_sender.send(format!("Logging into Rizon..."))?;
    while !has_joined {
        let message = match connection.read_message() {
            Ok(msg) => msg,
            _ => return Err(anyhow!("Error reading TcpStream"))
        };
        let now = time::Instant::now();
        if let Some(msg) = message {
            //println!("{}",msg);
            if msg.starts_with("PING :") {
                let pong = msg.replace("PING", "PONG");
                connection.socket.write(pong.as_bytes())?;
                if !has_joined {
                    let channel_join_cmd = format!("JOIN #{}\r\n", request.channel);
                    connection.socket.write(channel_join_cmd.as_bytes())?;
                }
            } else if MODE_REGEX.is_match(&msg) {
                if !has_joined {
                    let channel_join_cmd = format!("JOIN #{}\r\n", request.channel);
                    connection.socket.write(channel_join_cmd.as_bytes())?;
                }
            } else if msg.contains("JOIN :#") {
                has_joined = true;
            }
        } else if now >= next {
            let channel_join_cmd = format!("JOIN #{}\r\n", request.channel);
            connection.socket.write(channel_join_cmd.as_bytes())?;
            next = now + time::Duration::from_millis(500);
            timeout_counter += 1;
            if timeout_counter > timeout_threshold {
                return Err(anyhow!("Timed out logging in"))
            }
        }
        //thread::sleep(time::Duration::from_micros(10));
    }

    status_bar_sender.send(format!("Connected"))?;

    let mut i = 0;
    let mut requests : Vec<DCCSend> = vec![];
    let mut resume = false;
    let mut wait = false;
    let mut received_reply;
    while download_handles.len() < request.packages.len() && timeout_counter <= timeout_threshold {
        if wait {
            //wait til a previous package is downloaded then proceed
            let f = fs::File::open(&requests[i-1].filename)?;
            let meta = f.metadata()?;
            while meta.len() < requests[i-1].file_size as u64 {
                sleep(time::Duration::from_secs(1));
            }
            wait = false;
        }
        let package_bot = &request.bot[i];
        let package_number = &request.packages[i];
        if !resume {
            let xdcc_send_cmd =
                format!("PRIVMSG {} :xdcc send #{}\r\n", package_bot, package_number);
            connection.socket.write(xdcc_send_cmd.as_bytes())?;
        }

        next = time::Instant::now() + time::Duration::from_millis(3000);
        timeout_counter = 0;
        received_reply = false;
        while !received_reply && timeout_counter <= timeout_threshold {
            let message = match connection.read_message() {
                Ok(msg) => msg,
                _ => return Err(anyhow!("Error reading TcpStream on pack {}", package_number))
            };
            let now = time::Instant::now();
            if let Some(msg) = message {
                //println!("{}",msg);
                if DCC_SEND_REGEX.is_match(&msg) {
                    let request = parse_dcc_send(&msg)?;
                    requests.push(request);
                    status_bar_sender.send(format!("Now downloading {}", &requests[i].filename))?;
                    if std::path::Path::new(&requests[i].filename).exists() {
                        status_bar_sender.send(format!("Found an existing {}", &requests[i].filename))?;
                        let f = fs::File::open(&requests[i].filename)?;
                        let meta = f.metadata()?;
                        if (meta.len() as usize) < requests[i].file_size {
                            let xdcc_resume_cmd =
                                format!("PRIVMSG {} :\x01DCC RESUME \"{}\" {} {}\x01\r\n", package_bot, &requests[i].filename, &requests[i].port, meta.len());
                            connection.socket.write(xdcc_resume_cmd.as_bytes())?;
                            resume = true;
                        }
                    }
                    if !resume {
                        let req = requests[i].clone();
                        let sender = channel_senders[i].clone();
                        let path = dir_path.clone();
                        let status_bar_sender_clone = status_bar_sender.clone();
                        let handle = thread::spawn(move || {
                            download_file(req, sender, path).expect("Failed to download.");
                            status_bar_sender_clone.send("Episode Finished Downloading".to_string()).unwrap();
                        });
                        download_handles.push(handle);
                        i += 1;
                    }
                    received_reply = true;
                } else if resume && msg.contains("DCC ACCEPT ") {
                    status_bar_sender.send(format!("Attempting to resume download for {}", requests[i].filename))?;
                    let req = requests[i].clone();
                    let sender = channel_senders[i].clone();
                    let path = dir_path.clone();
                    let status_bar_sender_clone = status_bar_sender.clone();
                    let handle = thread::spawn(move || {
                        download_file(req, sender, path).expect("Failed to download.");
                        status_bar_sender_clone.send("Episode Finished Downloading".to_string()).unwrap();
                    });
                    download_handles.push(handle);
                    i += 1;
                    resume = false;
                    received_reply = true;
                } else if msg.contains(" queued too many ") {
                    //bot tells you that you can't queue up a new file
                    wait = true;
                    received_reply = true;
                } else if msg.contains("NOTICE ") && msg.ends_with(" You already requested") {
                    status_bar_sender.send(format!("A previous request was made for pack {}, attempting to cancel and retry", package_number))?;
                    let xdcc_remove_cmd =
                        format!("PRIVMSG {} :xdcc remove #{}\r\n", package_bot, package_number);
                    connection.socket.write(xdcc_remove_cmd.as_bytes())?;
                    let xdcc_cancel_cmd =
                        format!("PRIVMSG {} :\x01XDCC CANCEL\x01\r\n", package_bot);
                    connection.socket.write(xdcc_cancel_cmd.as_bytes())?;
                    received_reply = true;
                }
            } else {
                //postpone the timeout if currently downloading, if bot doesn't care to give queue message
                //some batch xdcc bots will add you into a queue but won't send more than x number of dcc sends
                let mut dl_in_progress = false;
                if (i > requests.len()) && std::path::Path::new(&requests[i-1].filename).exists() {
                    let f = fs::File::open(&requests[i - 1].filename)?;
                    let meta = f.metadata()?;

                    if !meta.len() < requests[i - 1].file_size as u64 {
                        dl_in_progress = true;
                    }
                }
                if now >= next && !dl_in_progress {
                    next = now + time::Duration::from_millis(3000);
                    timeout_counter += 1;
                    status_bar_sender.send(format!("({}/{}) Waiting on dcc send reply for pack {}...", timeout_counter, timeout_threshold, package_number))?;
                    if timeout_counter > timeout_threshold {
                        status_bar_sender.send(format!("Timed out receiving dcc send for pack {}", package_number))?;
                    }
                }
            }
        }
    }

    connection.socket.write("QUIT\r\n".as_bytes())?;
    connection.socket.shutdown(Shutdown::Both)?;
    download_handles
        .into_iter()
        .for_each(|handle| handle.join().expect("Failed to join thread."));
    status_bar_sender.send("Success".to_string())?;
    Ok(())
}

fn log_in(request: &IRCRequest) -> Result<TcpStream> {
    let mut stream = TcpStream::connect(&request.server)?;
    let mut rng = rand::thread_rng();
    let rng_num: u16 = rng.gen();
    let rng_nick = format!("{}{}", request.nickname, rng_num);
    stream.write(format!("NICK {}\r\n", rng_nick).as_bytes())?;
    stream.write(format!("USER {} 0 * {}\r\n", rng_nick, rng_nick).as_bytes())?;
    Ok(stream)
}

fn parse_dcc_send(message: &String) -> Result<DCCSend> {
    let captures = DCC_SEND_REGEX.captures(&message).ok_or_else(|| anyhow!("Failed to parse dcc request."))?;
    let ip_number = captures[2].parse::<u32>()?;
    Ok(DCCSend {
        filename: captures[1].to_string().replace("\"",""),
        ip: IpAddr::V4(Ipv4Addr::from(ip_number)),
        port: captures[3].to_string(),
        file_size: captures[4].parse::<usize>()?,
    })
}

fn download_file(
    request: DCCSend,
    sender: Sender<i64>,
    dir_path: PathBuf) -> Result<()> {
    let file_path = dir_path.join(&request.filename);
    let mut file = match fs::OpenOptions::new().append(true).open(file_path.clone()) {
        Ok(existing_file) => existing_file,
        _ => fs::File::create(file_path.clone())?
    };
    let mut stream = TcpStream::connect(format!("{}:{}", request.ip, request.port))?;
    let mut buffer = [0; 4096];
    let meta = file.metadata()?;
    let mut progress = meta.len() as usize;

    while progress < request.file_size {
        let count = stream.read(&mut buffer[..])?;
        file.write(&mut buffer[..count])?;
        progress += count;
        sender.send(progress as i64)?;
    }

    sender.send(-1)?;
    stream.shutdown(Shutdown::Both)?;
    file.flush()?;

    Ok(())
}
