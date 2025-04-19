use anyhow::{Context, Result, anyhow};
use clap::{Parser, Subcommand};
use log::{LevelFilter, info};
use std::collections::VecDeque;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    ViolentCleanup {
        #[arg(short, long)]
        directory: PathBuf,

        #[arg(short, long, default_values=[OsStr::new("mp4"), OsStr::new("jpg")])]
        filter_extensions: Vec<OsString>,

        #[arg(short, long)]
        target_use_percentage: u8,

        #[arg(long)]
        actually_rm: bool,
    },
}

fn main() -> Result<()> {
    pretty_env_logger::formatted_builder()
        .filter_level(LevelFilter::Info)
        .parse_default_env()
        .init();

    let args = Args::parse();

    match args.command {
        Command::ViolentCleanup {
            directory,
            filter_extensions,
            target_use_percentage,
            actually_rm,
        } => {
            let directory = directory.canonicalize()?;
            let use_percentage = read_use_percentage(&directory)?;
            info!("current: {use_percentage}%, target: {target_use_percentage}%");
            if use_percentage < target_use_percentage {
                return Ok(());
            }
            let mut matches = find_matching_files(&directory, filter_extensions)?;
            while let Some(candidate) = matches.pop_front() {
                let use_percentage = read_use_percentage(&directory)?;
                if use_percentage < target_use_percentage {
                    break;
                }
                info!("should remove: {:?}", candidate);
                if actually_rm {
                    fs::remove_file(&candidate)?;
                }
            }
            Ok(())
        }
    }
}

fn find_matching_files(
    directory: impl AsRef<Path>,
    filter_extensions: Vec<OsString>,
) -> Result<VecDeque<PathBuf>> {
    let mut matches = Vec::with_capacity(1024);
    for entry in walkdir::WalkDir::new(directory).into_iter() {
        match dir_entry_to_modified(entry) {
            Ok(Some((path, modified))) => {
                let ext = match path.extension() {
                    Some(ext) => ext,
                    None => continue,
                };
                if filter_extensions.iter().any(|e| e == ext) {
                    matches.push((path, modified));
                }
            }
            Ok(None) => continue,
            Err(e) => {
                log::debug!("error reading directory, ignoring: {e:?}");
                continue;
            }
        }
    }
    matches.sort_unstable_by_key(|(_, modified)| *modified);
    Ok(matches.into_iter().map(|(path, _)| path).collect())
}

fn read_use_percentage(directory: impl AsRef<Path>) -> Result<u8> {
    let stat = nix::sys::statvfs::statvfs(directory.as_ref())?;
    let use_percentage = u8::try_from(100u64 - (stat.blocks_free() * 100 / (stat.blocks())))?;
    Ok(use_percentage)
}

fn dir_entry_to_modified(
    entry: walkdir::Result<walkdir::DirEntry>,
) -> Result<Option<(PathBuf, u64)>> {
    let entry = entry?;
    if !entry.file_type().is_file() {
        return Ok(None);
    }

    let path = entry.path();

    let modified = get_file_modified(&entry).with_context(|| anyhow!("reading {:?}", &path))?;
    Ok(Some((path.to_path_buf(), modified)))
}

fn get_file_modified(entry: &walkdir::DirEntry) -> Result<u64> {
    Ok(entry
        .metadata()?
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs())
}
