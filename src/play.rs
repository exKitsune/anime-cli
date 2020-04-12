use std::path::PathBuf;
use std::thread;
use std::thread::JoinHandle;

#[cfg(feature = "with-mpv")]
pub fn play_video(filenames: Vec<String>, dir_path: PathBuf) -> JoinHandle<()> {
    thread::spawn(move || {
        thread::sleep(std::time::Duration::from_secs(5));
        let mut i = 0;
        let mut timeout = 0;
        let mut filename = &filenames[i];
        let video_path = dir_path.join(filename);
        while timeout < 5 { //Initial connection waiting
            if !video_path.is_file() {
                timeout += 1;
                thread::sleep(std::time::Duration::from_secs(5));
            } else {
                break;
            }
        }
        let mut mpv_builder = mpv::MpvHandlerBuilder::new().expect("Failed to init MPV builder");
        if video_path.is_file() {
            let video_path = video_path
                .to_str()
                .expect("Expected a string for Path, got None");
            mpv_builder.set_option("osc", true).unwrap();
            mpv_builder
                .set_option("input-default-bindings", true)
                .unwrap();
            mpv_builder.set_option("input-vo-keyboard", true).unwrap();
            let mut mpv = mpv_builder.build().expect("Failed to build MPV handler");
            mpv.command(&["loadfile", video_path as &str])
                .expect("Error loading file");
            'main: loop {
                while let Some(event) = mpv.wait_event(0.0) {
                    //println!("{:?}", event);
                    match event {
                        mpv::Event::Shutdown => {
                            break 'main;
                        }
                        mpv::Event::Idle => {
                            if i >= filenames.len() {
                                break 'main;
                            }
                        }
                        mpv::Event::EndFile(Ok(mpv::EndFileReason::MPV_END_FILE_REASON_EOF)) => {
                            i += 1;
                            if i >= filenames.len() {
                                break 'main;
                            }
                            filename = &filenames[i];
                            let next_video_path = dir_path.join(filename);
                            if next_video_path.is_file() {
                                let next_video_path = next_video_path
                                    .to_str()
                                    .expect("Expected a string for Path, got None");
                                mpv.command(&["loadfile", next_video_path as &str])
                                    .expect("Error loading file");
                            } else {
                                eprintln!(
                                    "A file is required; {} is not a valid file",
                                    next_video_path.to_str().unwrap()
                                );
                            }
                        }
                        _ => {}
                    };
                }
            }
        } else {
            eprintln!(
                "A file is required; {} is not a valid file",
                video_path.to_str().unwrap()
            );
        }
    })
}

#[cfg(feature = "with-opener")]
pub fn play_video(filenames: Vec<String>, dir_path: PathBuf) -> JoinHandle<()> {
    thread::spawn(move || {
        thread::sleep(std::time::Duration::from_secs(5));
        let filename = &filenames[0];
        let video_path = dir_path.join(filename);

        let mut timeout = 0;
        while timeout < 5 { //Initial connection waiting
            if !video_path.is_file() {
                timeout += 1;
                thread::sleep(std::time::Duration::from_secs(5));
            } else {
                break;
            }
        }
        if let Err(e) = opener::open(video_path) {
            eprintln!("{:?}", e);
        };
    })
}
