use std::{env, fs, path::Path};

use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};

const URL_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}');

pub fn detect_wsl_distro() -> Option<String> {
    let distro = env::var("WSL_DISTRO_NAME").ok()?;
    if distro.is_empty() {
        return None;
    }
    let osrelease = fs::read_to_string("/proc/sys/kernel/osrelease").ok()?;
    osrelease
        .to_lowercase()
        .contains("microsoft")
        .then_some(distro)
}

pub fn windows_path(linux_path: &Path, distro: &str) -> Option<String> {
    let path = linux_path.to_str()?;
    if let Some(drive_path) = windows_drive_path(path) {
        return Some(drive_path.replace('/', "\\"));
    }
    if !path.starts_with('/') || distro.is_empty() {
        return None;
    }
    Some(format!(
        "\\\\wsl.localhost\\{distro}{}",
        path.replace('/', "\\")
    ))
}

pub fn windows_file_url(linux_path: &Path, distro: &str) -> Option<String> {
    let path = linux_path.to_str()?;
    let raw_url = if let Some(drive_path) = windows_drive_path(path) {
        format!("file:///{drive_path}")
    } else {
        windows_path(linux_path, distro)?;
        format!("file://///wsl.localhost/{distro}{path}")
    };
    Some(utf8_percent_encode(&raw_url, URL_ENCODE_SET).to_string())
}

fn windows_drive_path(path: &str) -> Option<String> {
    let path = path.strip_prefix("/mnt/")?;
    let (drive, rest) = path.split_once('/').unwrap_or((path, ""));
    if !matches!(drive, "c" | "d" | "e" | "f") {
        return None;
    }
    Some(format!("{}:/{rest}", drive.to_ascii_uppercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_wsl_paths_and_urls() {
        let path = Path::new("/home/user/My Documents/note #1.txt");
        assert_eq!(
            windows_path(path, "Ubuntu").as_deref(),
            Some("\\\\wsl.localhost\\Ubuntu\\home\\user\\My Documents\\note #1.txt")
        );
        assert_eq!(
            windows_file_url(path, "Ubuntu").as_deref(),
            Some("file://///wsl.localhost/Ubuntu/home/user/My%20Documents/note%20%231.txt")
        );
    }

    #[test]
    fn converts_windows_drive_paths_before_wsl_paths() {
        for (drive, upper) in [('c', 'C'), ('d', 'D'), ('e', 'E'), ('f', 'F')] {
            let path = format!("/mnt/{drive}/Users/user/My Documents/note #1.txt");
            let path = Path::new(&path);
            assert_eq!(
                windows_path(path, "Ubuntu").as_deref(),
                Some(format!("{upper}:\\Users\\user\\My Documents\\note #1.txt").as_str())
            );
            assert_eq!(
                windows_file_url(path, "Ubuntu").as_deref(),
                Some(
                    format!("file:///{upper}:/Users/user/My%20Documents/note%20%231.txt").as_str()
                )
            );
        }
    }

    #[test]
    fn rejects_relative_paths() {
        assert!(windows_path(Path::new("note.txt"), "Ubuntu").is_none());
    }
}
