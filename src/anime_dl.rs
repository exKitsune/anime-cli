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
use pbr::{MultiBar, Pipe, ProgressBar, Units};
use std::sync::mpsc::{channel, Receiver};
use terminal_size::{Width, terminal_size};

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

pub fn connect_and_download(request: IRCRequest, dir_path: PathBuf) -> Result<()> {
    let mut channel_senders = vec![];
    let mut multi_bar = MultiBar::new();
    let mut multi_bar_handles = vec![];
    let (status_bar_sender, status_bar_receiver) = channel();
    let mut pb_message = String::new();

    println!("Connecting to Rizon...");

    let mut download_handles = Vec::new();
    let mut has_joined = false;
    let stream = log_in(&request)?;
    let mut connection : IRCConnection = IRCConnection { socket: stream, partial_msg: "".to_string()};

    let mut next = time::Instant::now() + time::Duration::from_millis(500);
    let timeout_threshold = 5;
    let mut timeout_counter = 0;
    println!("Logging into Rizon...");
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

    println!("Connected!");

    let mut i = 0;
    let mut requests : Vec<DCCSend> = vec![];
    let mut resume = false;
    let mut wait = false;
    let mut catch_pak = false;
    while download_handles.len() < request.packages.len() && timeout_counter <= timeout_threshold {
        if catch_pak {
            let (sender, receiver) = channel();
            let handle;

            match terminal_size() {
                Some((Width(w), _)) if requests[i].filename.len() > w as usize / 2 => { // trim the filename
                    let filename = &requests[i].filename;
                    let acceptable_length = w as usize / 2;
                    let first_half = &filename[..filename.char_indices().nth(acceptable_length/2).unwrap().0];
                    let second_half = &filename[filename.char_indices().nth_back(acceptable_length/2).unwrap().0..];
                    if acceptable_length < 50 {
                        pb_message.push_str(first_half);
                    }
                    pb_message.push_str("...");
                    pb_message.push_str(second_half);
                },
                _ => pb_message.push_str(&requests[i].filename)
            };
            pb_message.push_str(": ");

            let mut progress_bar = multi_bar.create_bar(requests[i].file_size as u64);
            progress_bar.set_units(Units::Bytes);
            progress_bar.message(&pb_message);
            pb_message.clear();

            handle = thread::spawn(move || { // create an individual thread for each bar in the multibar with its own i/o
                update_bar(&mut progress_bar, receiver);
            });

            channel_senders.push(sender);
            multi_bar_handles.push(handle);

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
            catch_pak = false;
            continue
        }
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
        while timeout_counter <= timeout_threshold {
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
                    if std::path::Path::new(&requests[i].filename).exists() {
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
                        catch_pak = true;
                    }
                    break;
                } else if resume && msg.contains("DCC ACCEPT ") {
                    catch_pak = true;
                    resume = false;
                    break;
                } else if msg.contains(" queued too many ") {
                    //bot tells you that you can't queue up a new file
                    wait = true;
                    break;
                } else if msg.contains("NOTICE ") && msg.ends_with(" You already requested") {
                    let xdcc_remove_cmd =
                        format!("PRIVMSG {} :xdcc remove #{}\r\n", package_bot, package_number);
                    connection.socket.write(xdcc_remove_cmd.as_bytes())?;
                    let xdcc_cancel_cmd =
                        format!("PRIVMSG {} :\x01XDCC CANCEL\x01\r\n", package_bot);
                    connection.socket.write(xdcc_cancel_cmd.as_bytes())?;
                    break;
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
                }
            }
        }
    }

    connection.socket.write("QUIT\r\n".as_bytes())?;
    connection.socket.shutdown(Shutdown::Both)?;

    
    let mut status_bar = multi_bar.create_bar(requests.len() as u64);
    status_bar.set_units(Units::Default);
    status_bar.message(&format!("{}: ", "Waiting..."));
    let status_bar_handle = thread::spawn(move || {
        update_status_bar(&mut status_bar, status_bar_receiver);
    });
    multi_bar_handles.push(status_bar_handle);

    let _ = thread::spawn(move || { // multi bar listen is blocking
        multi_bar.listen();
    });

    download_handles
        .into_iter()
        .for_each(|handle| handle.join().expect("Failed to join thread."));
    status_bar_sender.send("Success".to_string())?;
    std::thread::sleep(std::time::Duration::from_secs(1));
    multi_bar_handles.into_iter().for_each(|handle| handle.join().unwrap());
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

fn update_status_bar(progress_bar: &mut ProgressBar<Pipe>, receiver: Receiver<String>) {
    while let Ok(progress) = receiver.recv() {
        progress_bar.message(&format!("{} ", progress));
        match progress.as_str() {
            "Episode Finished Downloading" => { progress_bar.inc(); },
            "Success" => break,
            _ => progress_bar.tick()
        }
    }
    progress_bar.tick();
    progress_bar.finish()
}

fn update_bar(progress_bar: &mut ProgressBar<Pipe>, receiver: Receiver<i64>) {
    while let Ok(progress) = receiver.recv() {
        if progress > 0 {
            progress_bar.set(progress as u64);
        } else {
            progress_bar.tick();
            return progress_bar.finish();
        }
    };
}