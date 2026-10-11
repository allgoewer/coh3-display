use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use std::time::{Duration, SystemTime};

use crate::log_parser::{LogEvent, MatchAssembler};

/// Follows `warnings.log`, returning the matches that appeared since the last poll.
#[derive(Debug)]
pub struct LogTail {
    path: PathBuf,
    offset: u64,
    /// Bytes read past the last newline, i.e. a line the game has not finished writing.
    partial: Vec<u8>,
    assembler: MatchAssembler,
}

impl LogTail {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            offset: 0,
            partial: Vec::new(),
            assembler: MatchAssembler::new(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn reset(&mut self) {
        self.offset = 0;
        self.partial.clear();
        self.assembler = MatchAssembler::new();
    }

    /// How long ago the game last wrote to the log. `None` if that cannot be told.
    pub fn quiet_for(&self) -> Option<Duration> {
        let modified = std::fs::metadata(&self.path).ok()?.modified().ok()?;
        SystemTime::now().duration_since(modified).ok()
    }

    /// Reads whatever was appended since the last call. The first call scans the whole
    /// file. A missing file is not an error: the game just has not been started yet.
    pub fn poll(&mut self) -> io::Result<Vec<LogEvent>> {
        let mut file = match File::open(&self.path) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                let was_read = self.offset > 0;
                self.reset();
                return Ok(if was_read { vec![LogEvent::Closed] } else { Vec::new() });
            }
            Err(err) => return Err(err),
        };

        let len = file.metadata()?.len();
        let mut matches = Vec::new();
        if len < self.offset {
            // The game recreates the log on every launch, so the run it had logged is over.
            self.reset();
            matches.push(LogEvent::Closed);
        }
        if len == self.offset {
            return Ok(matches);
        }

        file.seek(SeekFrom::Start(self.offset))?;
        let mut appended = Vec::new();
        file.take(len - self.offset).read_to_end(&mut appended)?;
        self.offset += appended.len() as u64;
        self.partial.extend_from_slice(&appended);

        let Some(last_newline) = self.partial.iter().rposition(|&b| b == b'\n') else {
            return Ok(matches);
        };
        let complete: Vec<u8> = self.partial.drain(..=last_newline).collect();

        // Drop the final newline so splitting does not produce a phantom empty last line.
        for line in complete[..last_newline].split(|&b| b == b'\n') {
            let line = String::from_utf8_lossy(line);
            if let Some(found) = self.assembler.feed(line.trim_end_matches('\r')) {
                matches.push(found);
            }
        }
        Ok(matches)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, OpenOptions};
    use std::io::Write;

    use crate::model::LoggedMatch;

    const LOG: &str = include_str!("../tests/fixtures/warnings_excerpt.log");

    fn temp_log(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("coh3-display-{}-{name}.log", std::process::id()))
    }

    fn append(path: &Path, text: &str) {
        let mut file = OpenOptions::new().create(true).append(true).open(path).unwrap();
        file.write_all(text.as_bytes()).unwrap();
    }

    fn started(tail: &mut LogTail) -> Vec<LoggedMatch> {
        let events = tail.poll().unwrap();
        events
            .into_iter()
            .filter_map(|event| match event {
                LogEvent::Started(started) => Some(started),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn missing_file_yields_nothing() {
        let mut tail = LogTail::new(temp_log("missing"));
        assert!(tail.poll().unwrap().is_empty());
    }

    #[test]
    fn follows_appends_and_waits_for_complete_lines() {
        let path = temp_log("append");
        let _ = fs::remove_file(&path);
        fs::write(&path, LOG).unwrap();

        let mut tail = LogTail::new(&path);
        assert_eq!(started(&mut tail).len(), 2);
        assert!(tail.poll().unwrap().is_empty());

        append(&path, "(I) [23:00:23.432] [000015420]: GAME -- Scenario: data:scenarios\\multiplayer\\pisa_centrale_4p\\pisa_centrale_4p\n");
        append(&path, "(I) [23:00:23.432] [000015420]: GAME -- Human Player: 0 Fight Fate 917165 0 americans\n");
        // Half-written line: must not close the block or be parsed yet.
        append(&path, "(I) [23:00:23.432] [000015420]: GAME -- Human Player: 1 Tropiciel_Pawe");
        assert!(tail.poll().unwrap().is_empty());

        append(&path, "Ł 1306328 1 germans\n(I) [23:00:23.433] [000015420]: GameApp::SetState : new (LoadingGame) old (UnloadedGame)\n");
        let matches = started(&mut tail);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].map(), "pisa_centrale_4p");
        assert_eq!(matches[0].players[1].name, "Tropiciel_PaweŁ");
        assert_eq!(matches[0].team_size(), 1);

        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn restarts_when_the_log_is_recreated() {
        let path = temp_log("truncate");
        let _ = fs::remove_file(&path);
        fs::write(&path, LOG).unwrap();

        let mut tail = LogTail::new(&path);
        assert_eq!(started(&mut tail).len(), 2);

        let first_match: String = LOG.lines().skip(10).take(8).map(|l| format!("{l}\n")).collect();
        fs::write(&path, first_match).unwrap();
        let events = tail.poll().unwrap();
        // A new log means the run of the game that wrote the old one is over.
        assert_eq!(events.first(), Some(&LogEvent::Closed));
        let Some(LogEvent::Started(started)) = events.last() else {
            panic!("expected the match, got {events:?}");
        };
        assert_eq!(events.len(), 2);
        assert_eq!(started.map(), "elst_outskirts_4p");

        fs::remove_file(&path).unwrap();
    }
}
