//! the console writer, which is the one thing on the host that sends to the board rather than reading from it.
//!
//! the firmware reads single characters off USART1 and each one is a command, so a typo is a command too. every key is checked against the alphabet the firmware's switch actually implements before a byte goes out, and the board is asked afterwards what it now thinks it is, because a write nobody confirmed is not an arm.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

/// the console alphabet, read out of the switch in firmware/stm32u585/Src/app_threadx.c rather than out of the comment above it.
#[derive(Debug)]
struct Key {
    key: char,
    selects: &'static str,
    /// whether the key reboots the board through the word of SRAM4 that survives a warm reset.
    resets: bool,
    /// which status line answers for the key, or nothing when none of them does.
    confirms: Option<Confirm>,
}

/// one field of one of the board's once a second status lines.
///
/// the line is named as well as the field, because the exporter prints three of them and two could carry a field of the same name without meaning the same thing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Confirm {
    line: &'static str,
    field: &'static str,
    value: &'static str,
}

/// the status lines the exporter prints, in the order it prints them. the inference line answers for no key and is read because the golden verdict is on it.
const LINES: [&str; 4] = ["placement ", "stress ", "net ", "infer "];

const fn on(line: &'static str, field: &'static str, value: &'static str) -> Option<Confirm> {
    Some(Confirm { line, field, value })
}

const ALPHABET: &[Key] = &[
    Key { key: 'v', selects: "victim is the inference path", resets: false, confirms: on("stress ", "victim", "infer") },
    Key { key: 'r', selects: "victim is the read loop", resets: false, confirms: on("stress ", "victim", "read") },
    Key { key: 'i', selects: "victim is inference on the stress schedule", resets: false, confirms: on("stress ", "victim", "infer-stress") },
    Key { key: 't', selects: "no victim, the DMA transfer is timed", resets: false, confirms: on("stress ", "victim", "dma") },
    Key { key: '0', selects: "bandwidth sweep", resets: false, confirms: on("stress ", "sweep", "bw") },
    Key { key: '1', selects: "transactions at fixed bandwidth", resets: false, confirms: on("stress ", "sweep", "xact") },
    Key { key: '2', selects: "channel count at fixed bandwidth", resets: false, confirms: on("stress ", "sweep", "chan") },
    Key { key: '3', selects: "address stride", resets: false, confirms: on("stress ", "sweep", "stride") },
    Key { key: '4', selects: "past the top of the bandwidth sweep", resets: false, confirms: on("stress ", "sweep", "sat") },
    Key { key: '5', selects: "below the bandwidth sweep", resets: false, confirms: on("stress ", "sweep", "low") },
    Key { key: '6', selects: "one block moved once and timed", resets: false, confirms: on("stress ", "sweep", "dmat") },
    Key { key: 'o', selects: "no aggressor at all", resets: false, confirms: on("stress ", "sweep", "none") },
    Key { key: 'a', selects: "aggressor in SRAM1", resets: false, confirms: on("stress ", "region", "1") },
    Key { key: 'b', selects: "aggressor in SRAM2", resets: false, confirms: on("stress ", "region", "2") },
    Key { key: 'c', selects: "aggressor in SRAM3", resets: false, confirms: on("stress ", "region", "3") },
    Key { key: 'd', selects: "aggressor in SRAM4", resets: false, confirms: on("stress ", "region", "4") },
    Key { key: 'P', selects: "descriptors in SRAM1", resets: false, confirms: on("stress ", "desc", "1") },
    Key { key: 'Q', selects: "descriptors in SRAM2", resets: false, confirms: on("stress ", "desc", "2") },
    Key { key: 'R', selects: "descriptors in SRAM3", resets: false, confirms: on("stress ", "desc", "3") },
    Key { key: 'S', selects: "descriptors in SRAM4", resets: false, confirms: on("stress ", "desc", "4") },
    Key { key: '7', selects: "inference arena in SRAM1", resets: false, confirms: on("stress ", "arena", "1") },
    Key { key: '8', selects: "inference arena in SRAM2", resets: false, confirms: on("stress ", "arena", "2") },
    Key { key: '9', selects: "inference arena in SRAM3", resets: false, confirms: on("stress ", "arena", "3") },
    Key { key: 'X', selects: "read loop victim in SRAM1", resets: false, confirms: on("stress ", "vregion", "1") },
    Key { key: 'Y', selects: "read loop victim in SRAM2", resets: false, confirms: on("stress ", "vregion", "2") },
    Key { key: 'Z', selects: "read loop victim in SRAM3", resets: false, confirms: on("stress ", "vregion", "3") },
    Key { key: 'M', selects: "run the mem2mem aggressor through a stress pass", resets: false, confirms: on("stress ", "m2m", "1") },
    Key { key: 'N', selects: "stop the mem2mem aggressor again", resets: false, confirms: on("stress ", "m2m", "0") },
    // the stack is fixed when the thread is created, so these are requests for a reboot onto the page asked for. the placement line reports the source rather than the region, which is what tells the byte pool from the SRAM3 page, since the pool is in SRAM3.
    Key { key: 'j', selects: "stack in SRAM1, by reset", resets: true, confirms: on("placement ", "stack_src", "1") },
    Key { key: 'k', selects: "stack in SRAM2, by reset", resets: true, confirms: on("placement ", "stack_src", "2") },
    Key { key: 'n', selects: "stack in SRAM3, by reset", resets: true, confirms: on("placement ", "stack_src", "3") },
    Key { key: 'p', selects: "stack from the ThreadX byte pool, by reset", resets: true, confirms: on("placement ", "stack_src", "15") },
    // the ballast is taken before the measurement thread exists, so these reboot for the same reason, and the placement line reports the size the board came back with.
    Key { key: 'f', selects: "ballast 0 bytes, by reset", resets: true, confirms: on("placement ", "ballast", "0") },
    Key { key: 'g', selects: "ballast 64 bytes, by reset", resets: true, confirms: on("placement ", "ballast", "64") },
    Key { key: 'h', selects: "ballast 256 bytes, by reset", resets: true, confirms: on("placement ", "ballast", "256") },
    Key { key: 'm', selects: "ballast 1024 bytes, by reset", resets: true, confirms: on("placement ", "ballast", "1024") },
    Key { key: 'q', selects: "ballast 4096 bytes, by reset", resets: true, confirms: on("placement ", "ballast", "4096") },
    // the footprint moves vwords, which the board clamps to the window the region has, so what the line carries depends on the region and is not one value to check against.
    Key { key: 'A', selects: "victim footprint 1 KiB", resets: false, confirms: None },
    Key { key: 'B', selects: "victim footprint 2 KiB", resets: false, confirms: None },
    Key { key: 'C', selects: "victim footprint 4 KiB", resets: false, confirms: None },
    Key { key: 'D', selects: "victim footprint 8 KiB", resets: false, confirms: None },
    Key { key: 'E', selects: "victim footprint 16 KiB", resets: false, confirms: None },
    Key { key: 'F', selects: "victim footprint 32 KiB", resets: false, confirms: None },
    Key { key: 'G', selects: "victim footprint 64 KiB", resets: false, confirms: None },
    Key { key: 'H', selects: "victim footprint 128 KiB", resets: false, confirms: None },
    // vloads is vwords times vpasses, and vpasses is the requested count divided by vwords, so the line carries the request only when vwords divides it.
    Key { key: 'l', selects: "4096 loads per window", resets: false, confirms: None },
    Key { key: 'L', selects: "8192 loads per window", resets: false, confirms: None },
    Key { key: 'W', selects: "start the network stack", resets: false, confirms: on("net ", "started", "1") },
    Key { key: 'e', selects: "also send telemetry over UDP", resets: false, confirms: on("net ", "export", "1") },
];

/// the board polls the console once per exporter tick, which is one ThreadX tick, and reads one byte when it does. a burst would arrive faster than it is collected, so the keys are paced well clear of that.
const KEY_GAP: Duration = Duration::from_millis(50);

/// long enough for several of the board's once a second status sets, and for the reboot a stack or ballast key asks for.
const CONFIRM_FOR: Duration = Duration::from_secs(8);

/// what one arming answers with: the text a caller prints, and the board's own lines, so a caller that has to check a field no single key determines can read them rather than parse the text.
pub struct Armed {
    pub report: String,
    pub lines: Vec<String>,
}

pub fn run(args: &[String]) -> Result<Armed, String> {
    let mut keys: Option<String> = None;
    let mut allow_reset = false;
    for arg in args {
        match arg.as_str() {
            "--allow-reset" => allow_reset = true,
            other if other.starts_with('-') => return Err(format!("unknown option {other}")),
            other if keys.is_none() => keys = Some(other.to_string()),
            other => return Err(format!("unexpected argument {other}")),
        }
    }
    let keys = keys.ok_or_else(|| {
        "usage: laxity console <keys> [--allow-reset], for example laxity console i7c".to_string()
    })?;
    if keys.is_empty() {
        return Err("no keys given".to_string());
    }

    let written = resolve(&keys, allow_reset)?;
    // a reset key reboots the board, and everything the board was asked for that does not live in the word of SRAM4 comes back at its power on default. so the reboots go first whatever the caller wrote, and i7ck arms the cell it names instead of arming it and then throwing it away.
    let (resets, rest): (Vec<&Key>, Vec<&Key>) =
        written.iter().partition(|key| key.resets);
    let reordered = written
        .iter()
        .zip(resets.iter().chain(rest.iter()))
        .any(|(was, now)| was.key != now.key);

    let path = laxity_tui::find_serial_port()?;
    let mut port = serialport::new(path.to_string_lossy(), 921_600)
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::None)
        .stop_bits(serialport::StopBits::One)
        .flow_control(serialport::FlowControl::None)
        .timeout(Duration::from_millis(200))
        .open()
        .map_err(|error| format!("cannot open {}: {error}", path.display()))?;

    let mut out = String::new();
    out.push_str(&format!("port {}\n", path.display()));
    if reordered {
        let order: String = resets.iter().chain(rest.iter()).map(|key| key.key).collect();
        out.push_str(&format!(
            "sending {order} rather than {keys}, because a reset key returns everything not held in SRAM4 to its default and would discard the keys before it\n"
        ));
    }

    // the reboot is the reason this is two sends rather than one. a key that asks for one takes effect at once, and anything sent while the board is coming back is read by nobody, so the rest waits until the board reports the page it was asked for.
    if !resets.is_empty() {
        send(&mut *port, &resets, &mut out)?;
        let (lines, matched) = confirm(&mut *port, &wanted(&resets))?;
        report(&mut out, &lines, &resets);
        if !matched {
            return Err(format!("{out}the board did not come back on the page these keys ask for"));
        }
    }
    if !rest.is_empty() {
        send(&mut *port, &rest, &mut out)?;
    }

    let all: Vec<&Key> = resets.iter().chain(rest.iter()).copied().collect();
    let (lines, matched) = confirm(&mut *port, &wanted(&all))?;
    report(&mut out, &lines, &all);
    if !matched {
        return Err(format!(
            "{out}the board does not report the state these keys ask for, so nothing is armed"
        ));
    }
    Ok(Armed { report: out, lines })
}

/// wait for the board to classify the golden window correctly, which is the only end to end check this firmware has.
///
/// bench/sweeps/descriptor_free_run.sh waits for this before every capture and stops without it, since a board that has stopped classifying is a board whose cycles are still plausible. this matches on the same text the script greps for.
pub fn await_golden(seconds: u64) -> Result<String, String> {
    let path = laxity_tui::find_serial_port()?;
    let mut port = serialport::new(path.to_string_lossy(), 921_600)
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::None)
        .stop_bits(serialport::StopBits::One)
        .flow_control(serialport::FlowControl::None)
        .timeout(Duration::from_millis(200))
        .open()
        .map_err(|error| format!("cannot open {}: {error}", path.display()))?;

    let started = Instant::now();
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut last = String::new();
    while started.elapsed() < Duration::from_secs(seconds) {
        match port.read(&mut chunk) {
            Ok(0) => {}
            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
            Err(error) => return Err(format!("cannot read the board: {error}")),
        }
        while let Some((index, line)) = take_status(&mut buffer) {
            if LINES[index] != "infer " {
                continue;
            }
            if line.contains("MATCH mismatch=0") {
                return Ok(line);
            }
            last = line;
        }
    }
    if last.is_empty() {
        return Err(format!("the board printed no inference line in {seconds}s, so the golden check cannot be read"));
    }
    Err(format!("the board is not classifying the golden window: {last}"))
}

/// the fields the board has to report for a set of keys to have arrived.
fn wanted(keys: &[&Key]) -> Vec<Confirm> {
    keys.iter().filter_map(|key| key.confirms).collect()
}

fn send(port: &mut dyn Write, keys: &[&Key], out: &mut String) -> Result<(), String> {
    for key in keys {
        port.write_all(&[key.key as u8])
            .and_then(|()| port.flush())
            .map_err(|error| format!("cannot send {}: {error}", key.key))?;
        out.push_str(&format!("sent {}  {}\n", key.key, key.selects));
        std::thread::sleep(KEY_GAP);
    }
    Ok(())
}

/// the board's own lines, then what each key was looking for in them.
fn report(out: &mut String, lines: &[String], keys: &[&Key]) {
    for line in lines {
        out.push_str(&format!("board {line}\n"));
    }
    for key in keys {
        match key.confirms {
            Some(want) => out.push_str(&format!(
                "  {} selects {}={} on the {}line\n",
                key.key,
                want.field,
                want.value,
                want.line
            )),
            None => out.push_str(&format!(
                "  {} sent and not confirmable, because no status line reports it\n",
                key.key
            )),
        }
    }
}

/// every key looked up, with a typo named rather than sent.
fn resolve(keys: &str, allow_reset: bool) -> Result<Vec<&'static Key>, String> {
    let mut chosen = Vec::new();
    for key in keys.chars() {
        let Some(found) = ALPHABET.iter().find(|entry| entry.key == key) else {
            return Err(format!(
                "{key} is not a console key, so it would be sent as some other command; the alphabet is {}",
                ALPHABET.iter().map(|entry| entry.key).collect::<String>()
            ));
        };
        if found.resets && !allow_reset {
            return Err(format!(
                "{key} reboots the board, which selects {}; pass --allow-reset to mean it",
                found.selects
            ));
        }
        chosen.push(found);
    }
    Ok(chosen)
}

/// read the board's status lines until the set of them carries every field the keys ask for.
///
/// a status set already on the wire when the keys landed reports the state before them, so this waits for lines that agree rather than reading one and believing it. the latest of each line is kept, because the three are printed in sequence and a key answered by one of them does not care what the others said.
fn confirm(port: &mut dyn Read, wanted: &[Confirm]) -> Result<(Vec<String>, bool), String> {
    let started = Instant::now();
    let mut buffer: Vec<u8> = Vec::new();
    let mut latest: Vec<Option<String>> = vec![None; LINES.len()];
    let mut chunk = [0u8; 4096];
    while started.elapsed() < CONFIRM_FOR {
        match port.read(&mut chunk) {
            Ok(0) => {}
            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
            // the port carries framed binary as well as text, and a read that times out is an idle moment rather than a failure.
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
            Err(error) => return Err(format!("cannot read the board: {error}")),
        }
        while let Some((index, line)) = take_status(&mut buffer) {
            latest[index] = Some(line);
            if wanted.iter().all(|want| held(&latest, want)) {
                return Ok((seen(&latest), true));
            }
        }
    }
    let lines = seen(&latest);
    if lines.is_empty() {
        return Err("the board sent no status line, so there is nothing to confirm against".to_string());
    }
    Ok((lines, false))
}

/// whether the line that answers for one key carries the value it asks for.
fn held(latest: &[Option<String>], want: &Confirm) -> bool {
    let Some(index) = LINES.iter().position(|line| *line == want.line) else { return false };
    latest[index]
        .as_deref()
        .and_then(|line| field_of(line, want.field))
        .is_some_and(|value| value == want.value)
}

fn seen(latest: &[Option<String>]) -> Vec<String> {
    latest.iter().flatten().cloned().collect()
}

/// the next whole status line in the buffer, with which of the three it is, picked out of a stream that is mostly binary frames.
fn take_status(buffer: &mut Vec<u8>) -> Option<(usize, String)> {
    let (index, start) = LINES
        .iter()
        .enumerate()
        .filter_map(|(index, mark)| {
            buffer
                .windows(mark.len())
                .position(|window| window == mark.as_bytes())
                .map(|at| (index, at))
        })
        .min_by_key(|(_, at)| *at)?;
    let end = buffer[start..].iter().position(|byte| *byte == b'\r' || *byte == b'\n')?;
    let line = buffer[start..start + end].to_vec();
    buffer.drain(..start + end);
    // a frame that happened to carry a marker gives bytes that are not text. they are drained either way, so the next read makes progress rather than finding the same junk.
    if line.iter().all(|byte| byte.is_ascii_graphic() || *byte == b' ') {
        return Some((index, String::from_utf8_lossy(&line).into_owned()));
    }
    None
}

/// one `name=value` field of a status line, with the address a descriptor field carries in brackets left off.
pub fn field_of(line: &str, field: &str) -> Option<String> {
    line.split_whitespace().find_map(|token| {
        let (name, value) = token.split_once('=')?;
        (name == field).then(|| value.split('(').next().unwrap_or(value).to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str = "stress victim=infer-stress m2m=0 stack=1 arena=1 desc=3(0x20072264) vregion=1 vwords=1024 sweep=bw region=3 ok=1 point=2/5";

    #[test]
    fn a_character_outside_the_alphabet_is_named_and_not_sent() {
        let error = resolve("i7u", false).unwrap_err();
        assert!(error.starts_with("u is not a console key"), "{error}");
        // the message carries the alphabet, since the next thing anybody does is look for the key they meant.
        assert!(error.contains("v") && error.contains("7"));
    }

    /// a reset key returns everything not held in SRAM4 to its default, so it goes first however the caller wrote it.
    #[test]
    fn the_keys_that_reboot_go_first_whatever_order_they_were_written_in() {
        let written = resolve("i7ck", true).unwrap();
        let (resets, rest): (Vec<&Key>, Vec<&Key>) = written.iter().partition(|key| key.resets);
        let order: String = resets.iter().chain(rest.iter()).map(|key| key.key).collect();
        assert_eq!(order, "ki7c");
        // the order within each half is the caller's, so two reset keys or two selections keep the sequence they were written in.
        let written = resolve("7kic", true).unwrap();
        let (resets, rest): (Vec<&Key>, Vec<&Key>) = written.iter().partition(|key| key.resets);
        let order: String = resets.iter().chain(rest.iter()).map(|key| key.key).collect();
        assert_eq!(order, "k7ic");
    }

    /// the placement line carries the stack source rather than the region it landed in, which is the only thing that tells the byte pool from the SRAM3 page.
    #[test]
    fn the_placement_line_answers_for_the_keys_that_reboot() {
        const PLACEMENT: &str = "placement ok=1 span=0x20002000..0x20057000 s1=0x20002000 s2=0x20030000 s3=0x20040000 s2b=0x20039000 ctl=0x20002000 bytes=2944 stack_src=15 stack_addr=0x2005764c stack_region=3 ballast=1024 ballast_addr=0x20057000";
        assert_eq!(field_of(PLACEMENT, "stack_src").as_deref(), Some("15"));
        assert_eq!(field_of(PLACEMENT, "ballast").as_deref(), Some("1024"));
        // n and p both land the stack in SRAM3 and the source is what separates them.
        assert_eq!(field_of(PLACEMENT, "stack_region").as_deref(), Some("3"));
        let pool = wanted(&resolve("p", true).unwrap());
        let sram3 = wanted(&resolve("n", true).unwrap());
        assert_eq!(pool[0].value, "15");
        assert_eq!(sram3[0].value, "3");
        assert!(pool.iter().all(|c| c.line == "placement "));
    }

    /// ten keys are answered by none of the three lines, and each has a reason rather than an omission.
    #[test]
    fn the_unconfirmable_keys_are_the_footprints_and_the_load_counts() {
        let silent: String =
            ALPHABET.iter().filter(|key| key.confirms.is_none()).map(|key| key.key).collect();
        assert_eq!(silent, "ABCDEFGHlL");
    }

    #[test]
    fn a_key_that_reboots_the_board_needs_saying_so() {
        let error = resolve("j", false).unwrap_err();
        assert!(error.contains("reboots the board"), "{error}");
        assert_eq!(resolve("j", true).unwrap().len(), 1);
        // nine keys reboot, across the stack source and the ballast.
        let resetting: String =
            ALPHABET.iter().filter(|key| key.resets).map(|key| key.key).collect();
        assert_eq!(resetting, "jknpfghmq");
    }

    #[test]
    fn a_status_line_answers_for_the_keys_that_name_a_field() {
        assert_eq!(field_of(LINE, "victim").as_deref(), Some("infer-stress"));
        assert_eq!(field_of(LINE, "region").as_deref(), Some("3"));
        assert_eq!(field_of(LINE, "arena").as_deref(), Some("1"));
        // the descriptor field carries its address in brackets and the number in front of it is the region.
        assert_eq!(field_of(LINE, "desc").as_deref(), Some("3"));
        assert_eq!(field_of(LINE, "nothing"), None);

        let want = wanted(&resolve("i7c", false).unwrap());
        assert_eq!(
            want.iter().map(|c| (c.field, c.value)).collect::<Vec<_>>(),
            [("victim", "infer-stress"), ("arena", "1"), ("region", "3")]
        );
        assert!(want.iter().all(|c| field_of(LINE, c.field).as_deref() == Some(c.value)));
    }

    #[test]
    fn a_status_line_is_taken_out_of_a_stream_that_is_mostly_frames() {
        let mut buffer = Vec::new();
        buffer.extend_from_slice(&[0xFFu8, 0x00, b'L', b'X', 0x7F]);
        buffer.extend_from_slice(LINE.as_bytes());
        buffer.extend_from_slice(b"\r\n");
        buffer.extend_from_slice(&[0x01u8, 0x02]);
        assert_eq!(take_status(&mut buffer), Some((1, LINE.to_string())));
        assert_eq!(take_status(&mut buffer), None);
    }

    /// the table above is a copy of a switch in another language, and this tree has paid twice for two copies of one thing drifting apart. this reads the switch.
    #[test]
    fn the_table_is_exactly_the_switch_the_firmware_implements() {
        const FIRMWARE: &str =
            include_str!("../../../firmware/stm32u585/Src/app_threadx.c");
        let body = FIRMWARE
            .split_once("static void laxity_poll_console(void)")
            .expect("the console poller moved")
            .1
            .split_once("\n}")
            .expect("the console poller has no end")
            .0;
        let mut firmware: Vec<char> = body
            .match_indices("case '")
            .filter_map(|(at, _)| body[at + 6..].chars().next())
            .collect();
        let mut table: Vec<char> = ALPHABET.iter().map(|key| key.key).collect();
        let listed = table.len();
        firmware.sort_unstable();
        table.sort_unstable();
        table.dedup();
        assert_eq!(table.len(), listed, "a key is listed twice");
        assert_eq!(table, firmware, "the table and the firmware switch disagree");
    }
}
