extern crate regex;

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
	major : i8,
	minor : i8,
}

impl MajorMinor {
	fn to_string(&self) -> String {
		let mut s = String::new();
		s.push_str(self.major.to_string().as_str());
		s.push_str(":");
		s.push_str(self.minor.to_string().as_str());
		s
	}

	fn udev_path(&self) -> PathBuf {
		let mut filename = String::from("b");
		filename.push_str(self.to_string().as_str());
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
				major: caps.at(1).unwrap().parse::<i8>().unwrap(),
				minor: caps.at(2).unwrap().parse::<i8>().unwrap(),
			}
		}), "MajorMinor::from_str")
	}
}

#[derive(Debug)]
struct BlockMetadata {
	id_type : String,
	id_fs_type : Option<String>,
	id_fs_uuid : Option<String>,
}

#[derive(Debug)]
struct Partition {
	name : String,
	majmin : MajorMinor,
	size : Option<u64>,

	metadata : Option<BlockMetadata>,
}

#[derive(Debug)]
struct Block {
	name : String,
	majmin : MajorMinor,
	size : Option<u64>,
	partitions : Vec<Partition>,
}

fn parse_block_file<T: FromStr>(path : &Path, filename : &str) -> Option<T> {
	let filepath = PathBuf::from(path).join(filename);
	let mut file = none!(File::open(filepath));

	let ref mut contents = String::new();
	let _ = file.read_to_string(contents).unwrap();
	T::from_str(contents.trim()).ok()
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
			let majmin = parse_block_file(entry_path, "dev");

			if majmin.is_none() {
				continue
			}

			let majmin = majmin.unwrap();

			let size = parse_block_file(entry_path, "size");
			let meta = load_uevent_metadata(&majmin);
			ps.push(Partition { name: entry_name, majmin: majmin, size: size, metadata: meta })
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
			let size : Option<u64> = parse_block_file(path, "size");
			let parts = read_partitions(path, &name);
			Some(Block { name: name, majmin: majmin, size: size, partitions: parts })
		},
		_ => None,
	}
}

#[derive(Debug)]
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

fn load_uevent_metadata(device : &MajorMinor) -> Option<BlockMetadata> {
	let path = device.udev_path();
	let mut file = none!(File::open(path));
	let ref mut contents = String::new();
	let _ = none!(file.read_to_string(contents));
	parse_uevent_metadata(contents)
}

struct Row<'a> {
	name: String,
	majmin: String,
	size: String,
	row_type: &'a str,
}

fn format_major_minor(majmin: &MajorMinor) -> String {
	return format!("{:>3}:{:<3}", majmin.major, majmin.minor);
}

fn pretty_size(size: Option<u64>) -> String {
	return match size {
		Some(size) => format!("{}", size),
		None => "?".into(),
	}
}

fn print_blocks(blocks : Vec<Block>) {
	let mut rows = Vec::new();

	for block in blocks {
		rows.push(Row {
			name: block.name.to_owned(),
			majmin: format_major_minor(&block.majmin),
			size: pretty_size(block.size),
			row_type: "disk",
		});

		for part in block.partitions {
			let mut name = String::from("\u{2514}\u{2500}");
			name.push_str(part.name.as_str());
			rows.push(Row {
				name: name,
				majmin: format_major_minor(&part.majmin),
				size: pretty_size(part.size),
				row_type: "part",
			});
		}
	}

	let mut name_len = 0;
	let mut size_len = 0;
	for row in rows.as_slice() {
		name_len = std::cmp::max(name_len, row.name.len());
		size_len = std::cmp::max(size_len, row.size.len());
	}

	for row in rows {
		println!("{1:<0$} {2} {4:>3$} {5:<4}",
			name_len, row.name,
			row.majmin,
			size_len, row.size,
			row.row_type
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
