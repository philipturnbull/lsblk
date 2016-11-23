extern crate regex;

use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::fs::DirEntry;
use std::fs::File;
use std::io::Error;
use std::io::Read;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use regex::Regex;
use std::str::FromStr;

macro_rules! invalid {
	($x:expr, $msg:expr) => ($x.ok_or(Error::new(ErrorKind::InvalidData, $msg)))
}

macro_rules! none {
	($x:expr) => (match $x {
		Ok(y) => y,
		_ => return None
	})
}

#[derive(Debug)]
struct MajorMinor {
	major : u8,
	minor : u8,
}

impl MajorMinor {
	fn to_string(&self) -> String {
		let mut s = String::new();
		s.push_str(&self.major.to_string());
		s.push_str(":");
		s.push_str(&self.minor.to_string());
		s
	}

	fn udev_path(&self) -> PathBuf {
		let mut filename = String::from("b");
		filename.push_str(&self.to_string());
		let mut path = PathBuf::from("/run/udev/data");
		path.push(filename);
		path
	}
}

impl FromStr for MajorMinor {
	type Err = Error;
	fn from_str(s: &str) -> Result<MajorMinor, Error> {
		let re = Regex::new(r"^([0-9]+):([0-9]+)$").unwrap();

		invalid!(re.captures(s).map(|caps| {
			MajorMinor {
				major: caps.at(1).unwrap().parse::<u8>().unwrap(),
				minor: caps.at(2).unwrap().parse::<u8>().unwrap(),
			}
		}), "MajorMinor::from_str")
	}
}

#[derive(Debug)]
#[derive(PartialEq)]
struct BlockMetadata {
	id_type : String,
	id_fs_type : Option<String>,
	id_fs_uuid : Option<String>,
}

#[derive(Debug)]
struct Partition {
	name : String,
	majmin : MajorMinor,
	removable : Option<u64>,
	size : Option<u64>,
	readonly : Option<u64>,

	metadata : Option<BlockMetadata>,
	mountpoint : String,
}

#[derive(Debug)]
struct Block {
	name : String,
	majmin : MajorMinor,
	removable : Option<u64>,
	size : Option<u64>,
	readonly : Option<u64>,
	partitions : Vec<Partition>,
	mountpoint : String,
}

fn parse_block_file<T: FromStr>(path : &Path, filename : &str) -> Option<T> {
	let filepath = PathBuf::from(path).join(filename);
	let mut file = none!(File::open(filepath));

	let contents = &mut String::new();
	let _ = file.read_to_string(contents).unwrap();
	T::from_str(contents.trim()).ok()
}

fn parse_sector_file(path : &Path, filename : &str) -> Option<u64> {
	parse_block_file::<u64>(path, filename).map(|x| x*512)
}

fn parse_proc_mounts_line(line : &str) -> Option<(String, String)> {
	let re = Regex::new(r"^([^ ]+) ([^ ]+) .+$").unwrap();

	re.captures(line).map(|caps| {
		(caps.at(1).unwrap().to_owned(), caps.at(2).unwrap().to_owned())
	})
}

fn parse_proc_mounts() -> Option<HashMap<String, String>> {
	let mut file = none!(File::open("/proc/mounts"));
	let contents = &mut String::new();
	let _ = none!(file.read_to_string(contents));

	let mounts =
		contents.lines().filter_map(parse_proc_mounts_line).collect();

	Some(mounts)
}

fn parse_proc_swaps_line(line : &str) -> Option<String> {
	let re = Regex::new(r"^(/[^ ]+) +.+$").unwrap();

	re.captures(line).map(|caps| {
		caps.at(1).unwrap().to_owned()
	})
}

fn parse_proc_swaps() -> Option<HashSet<String>> {
	let mut file = none!(File::open("/proc/swaps"));
	let contents = &mut String::new();
	let _ = none!(file.read_to_string(contents));

        let swaps =
		contents.lines().filter_map(parse_proc_swaps_line).collect();

	Some(swaps)
}

fn read_partition_mountpoint(name : &str) -> String {
	let mut path = String::from("/dev/");
	path.push_str(name);
	let mounts = parse_proc_mounts().unwrap();
	match mounts.get(&path) {
		Some(mount) => mount.to_owned(),
		None => {
			let swaps = parse_proc_swaps().unwrap();
			String::from(if swaps.contains(&path) {
				"[SWAP]"
			} else {
				""
			})
		}
	}
}

fn read_partitions(path : &Path, block_name : &str) -> Vec<Partition> {
	let mut ps = Vec::new();
	let entries = fs::read_dir(path).unwrap();
	for entry in entries {
		let entry = entry.unwrap();
		let entry_path = entry.path();
		let entry_path = entry_path.as_path();
		let entry_name = entry.file_name();
		let entry_name = entry_name.to_string_lossy().into_owned();
		if entry_name.starts_with(block_name) {
			let removable = parse_block_file(entry_path, "removable");
			let majmin = parse_block_file(entry_path, "dev");

			if majmin.is_none() {
				continue
			}

			let majmin = majmin.unwrap();

			let size = parse_sector_file(entry_path, "size");
			let readonly = parse_block_file(entry_path, "ro");
			let meta = load_uevent_metadata(&majmin);
			let mountpoint = read_partition_mountpoint(&entry_name);
			ps.push(Partition { name: entry_name, removable: removable, majmin: majmin, size: size, readonly: readonly, metadata: meta, mountpoint: mountpoint })
		}
	}
	ps
}

fn read_block(dir : DirEntry) -> Option<Block> {
	let path = dir.path();
	let path = path.as_path();
	let name = dir.file_name();
	let name = name.to_string_lossy().into_owned();
	let majmin : Option<MajorMinor> = parse_block_file(path, "dev");
	match majmin {
		Some(majmin) => {
			let removable = parse_block_file(path, "removable");
			let size = parse_sector_file(path, "size");
			let readonly = parse_block_file(path, "ro");
			let parts = read_partitions(path, &name);
			let mountpoint = String::from("");
			Some(Block { name: name, removable: removable, majmin: majmin, size: size, readonly: readonly, partitions: parts, mountpoint: mountpoint })
		},
		_ => None,
	}
}

#[derive(Debug)]
#[derive(PartialEq)]
struct KeyValue<'a> {
	key : &'a str,
	value : &'a str,
}

fn parse_line(line : &str) -> Option<KeyValue> {
	let re = Regex::new(r"^E:([^=]+)=([^=]+)$").unwrap();

	re.captures(line).map(|caps| {
		KeyValue { key : caps.at(1).unwrap(), value : caps.at(2).unwrap() }
	})
}

#[test]
fn test_parse_line() {
	assert!(parse_line("E:ID_ATA_FEATURE_SET_PM=1") ==
		Some(KeyValue { key:"ID_ATA_FEATURE_SET_PM", value: "1"}));

	assert!(parse_line("E:KEY=one two three") ==
		Some(KeyValue { key:"KEY", value: "one two three"}));

	assert!(parse_line("W:12") == None);
	assert!(parse_line("E:ID_ATA_FEATURE_SET_PM") == None);
	assert!(parse_line("E:ID_ATA_FEATURE_SET_PM=1=1") == None);
}

fn parse_uevent_metadata(data : &str) -> Option<BlockMetadata> {
	let mut id_type = None;
	let mut id_fs_type = None;
	let mut id_fs_uuid = None;

	for kv in data.lines().map(parse_line) {
		match kv {
			Some(KeyValue { key:"ID_TYPE", value }) => {
				id_type = Some(value.to_owned())
			},
			Some(KeyValue { key:"ID_FS_TYPE", value }) => {
				id_fs_type = Some(value.to_owned())
			},
			Some(KeyValue { key:"ID_FS_UUID", value }) => {
				id_fs_uuid = Some(value.to_owned())
			},
			_ => {}
		}
	}

	match (id_type, id_fs_type, id_fs_uuid) {
		(Some(id_type), id_fs_type, id_fs_uuid) =>
			Some(BlockMetadata {
				id_type : id_type,
				id_fs_type : id_fs_type,
				id_fs_uuid : id_fs_uuid,
			}),
		_ => None
	}
}

#[test]
fn test_parse_uevent_metadata() {
	assert!(
		parse_uevent_metadata("E:ID_TYPE=disk") ==
		Some(BlockMetadata {
			id_type: "disk".to_string(),
			id_fs_type: None,
			id_fs_uuid: None,
		})
	);

	assert!(
		parse_uevent_metadata("E:ID_TYPE=disk\nE:ID_FS_TYPE=ext4") ==
		Some(BlockMetadata {
			id_type: "disk".to_string(),
			id_fs_type: Some("ext4".to_string()),
			id_fs_uuid: None,
		})
	);

	assert!(
		parse_uevent_metadata("E:ID_FS_TYPE=ext4") == None
	);

	assert!(
		parse_uevent_metadata("E:ID_TYPE=disk\nE:ID_FS_UUID=eca1e7f9-42c7-49b7-9f42-bec0c3e975e6") ==
		Some(BlockMetadata {
			id_type: "disk".to_string(),
			id_fs_type: None,
			id_fs_uuid: Some("eca1e7f9-42c7-49b7-9f42-bec0c3e975e6".to_string()),
		})
	);
}

fn load_uevent_metadata(device : &MajorMinor) -> Option<BlockMetadata> {
	let path = device.udev_path();
	let mut file = none!(File::open(path));
	let contents = &mut String::new();
	let _ = none!(file.read_to_string(contents));
	parse_uevent_metadata(contents)
}

enum BlockType { Disk, Partition }

fn describe_block_type(blocktype : BlockType) -> &'static str {
	match blocktype {
		BlockType::Disk => "disk",
		BlockType::Partition => "part",
	}
}

struct Row {
	name: String,
	majmin: String,
	removable: &'static str,
	size: String,
	readonly: &'static str,
	row_type: BlockType,
	mountpoint : String,
}

fn format_major_minor(majmin: &MajorMinor) -> String {
	return format!("{:>3}:{:<3}", majmin.major, majmin.minor);
}

#[test]
fn test_format_major_minor() {
	assert!(format_major_minor(&MajorMinor { major:   1, minor:   0 }) == "  1:0  ");
	assert!(format_major_minor(&MajorMinor { major:  10, minor:   0 }) == " 10:0  ");
	assert!(format_major_minor(&MajorMinor { major:   1, minor:  20 }) == "  1:20 ");
	assert!(format_major_minor(&MajorMinor { major: 100, minor:  20 }) == "100:20 ");
	assert!(format_major_minor(&MajorMinor { major: 100, minor: 200 }) == "100:200");
}

fn pretty_removable(removable : Option<u64>) -> &'static str {
	match removable {
		Some(0) => " 0",
		Some(_) => " 1",
		None => "  ",
	}
}

fn pretty_units(size : u64, power : u32, precision : usize, suffix : &str) -> String {
	let divisor = (1024 as u64).pow(power) as f64;
	let n = (size as f64) / divisor;

	format!("{0:>4.1$}{2}", n, precision, suffix)
}

fn pretty_size(size: Option<u64>) -> String {
	match size {
		Some(size) => match size {
			size if size < 1024 => format!("{:>5}", size),
			size if size <= ((1024 as u64).pow(2)) => pretty_units(size, 1, 0, "K"),
			size if size <= ((1024 as u64).pow(3)) => pretty_units(size, 2, 1, "M"),
			size if size <= ((1024 as u64).pow(4)) => pretty_units(size, 3, 0, "G"),
			size if size <= ((1024 as u64).pow(5)) => pretty_units(size, 4, 0, "T"),
			size if size <= ((1024 as u64).pow(6)) => pretty_units(size, 5, 0, "P"),
			size if size <= ((1024 as u64).pow(7)) => pretty_units(size, 6, 0, "E"),
			size if size <= ((1024 as u64).pow(8)) => pretty_units(size, 7, 0, "Z"),
			_ => "big".into(),
		},
		None => "     ".into(),
	}
}

#[test]
fn test_pretty_size() {
	assert!("     " == pretty_size(None));
	assert!(" 1023" == pretty_size(Some(1023)));
	assert!("   1K" == pretty_size(Some(1024)));
	assert!("57.3M" == pretty_size(Some(60063744)));
	assert!("   4G" == pretty_size(Some(4292870144)));
	assert!("  28G" == pretty_size(Some(30063722496)));
	assert!("  32G" == pretty_size(Some(34359738368)));
}

fn pretty_readonly(readonly: Option<u64>) -> &'static str {
	match readonly {
		Some(0) => " 0",
		Some(_) => " 1",
		None => "  ",
	}
}

#[test]
fn test_pretty_readonly() {
	assert!("  " == pretty_readonly(None));
	assert!(" 0" == pretty_readonly(Some(0)));
	assert!(" 1" == pretty_readonly(Some(1)));
	assert!(" 1" == pretty_readonly(Some(2)));
	assert!(" 1" == pretty_readonly(Some(1234)));
}

fn print_blocks(blocks : Vec<Block>) {
	let mut rows = Vec::new();

	for block in blocks {
		rows.push(Row {
			name: block.name.to_owned(),
			majmin: format_major_minor(&block.majmin),
			removable: pretty_removable(block.removable),
			size: pretty_size(block.size),
			readonly: pretty_readonly(block.readonly),
			row_type: BlockType::Disk,
			mountpoint: block.mountpoint.to_owned(),
		});

		for (i, part) in block.partitions.iter().enumerate() {
			let mut name = if i+1 == block.partitions.len() {
				String::from("\u{2514}\u{2500}")
			} else {
				String::from("\u{251C}\u{2500}")
			};
			name.push_str(&part.name);
			rows.push(Row {
				name: name,
				majmin: format_major_minor(&part.majmin),
				removable: pretty_removable(block.removable),
				size: pretty_size(part.size),
				readonly: pretty_readonly(part.readonly),
				row_type: BlockType::Partition,
				mountpoint: part.mountpoint.to_owned(),
			});
		}
	}

	let mut name_len = 0;
	for row in &rows[..] {
		name_len = std::cmp::max(name_len, row.name.chars().count());
	}


	println!("{1:<0$} MAJ:MIN RM  SIZE RO TYPE MOUNTPOINT", name_len, "NAME");
	for row in rows {
		println!("{1:<0$} {2} {3} {4:>5} {5} {6:<4} {7}",
			name_len, row.name,
			row.majmin,
			row.removable,
			row.size,
			row.readonly,
			describe_block_type(row.row_type),
			row.mountpoint,
		);
	}
}

fn main() {
	let block_root = Path::new("/sys/block");
	let block_dirs = fs::read_dir(block_root).unwrap();
	let blocks = block_dirs.filter_map(|dir| {
		dir.ok().map(read_block)
	}).filter_map(|block| block).collect::<Vec<_>>();
	print_blocks(blocks);
}
