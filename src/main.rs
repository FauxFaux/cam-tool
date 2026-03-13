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
            let bytes_to_free = compute_bytes_to_free(&directory, target_use_percentage)?;
            info!(
                "current: {use_percentage}%, target: {target_use_percentage}%, need to free: {:.1}MB",
                mb(bytes_to_free),
            );
            if use_percentage < target_use_percentage {
                return Ok(());
            }
            let mut matches = find_matching_files(&directory, filter_extensions)?;
            let mut freed_bytes_estimate: u64 = 0;
            while let Some((candidate, file_size)) = matches.pop_front() {
                let use_percentage = read_use_percentage(&directory)?;
                if use_percentage < target_use_percentage || freed_bytes_estimate >= bytes_to_free {
                    break;
                }
                info!(
                    "should remove: {:?} ({:.1} MB)",
                    candidate,
                    (file_size as f64) / 1024. / 1024.
                );
                if actually_rm {
                    fs::remove_file(&candidate)?;
                }
                freed_bytes_estimate += file_size;

                sync_all_the_way_down(&candidate)?;
            }
            info!("freed around {:.1}MB", mb(freed_bytes_estimate));
            Ok(())
        }
    }
}

fn mb(v: u64) -> f64 {
    (v as f64) / 1024. / 1024.
}

fn sync_all_the_way_down(starting_file: impl AsRef<Path>) -> Result<()> {
    let mut current = starting_file.as_ref();
    while let Some(parent) = current.parent() {
        fs::File::open(parent)?.sync_all()?;
        current = parent;
    }
    Ok(())
}

fn find_matching_files(
    directory: impl AsRef<Path>,
    filter_extensions: Vec<OsString>,
) -> Result<VecDeque<(PathBuf, u64)>> {
    let mut matches = Vec::with_capacity(1024);
    for entry in walkdir::WalkDir::new(directory).into_iter() {
        match dir_entry_to_modified(entry) {
            Ok(Some((path, modified, size))) => {
                let ext = match path.extension() {
                    Some(ext) => ext,
                    None => continue,
                };
                if filter_extensions.iter().any(|e| e == ext) {
                    matches.push((path, modified, size));
                }
            }
            Ok(None) => continue,
            Err(e) => {
                log::debug!("error reading directory, ignoring: {e:?}");
                continue;
            }
        }
    }
    matches.sort_unstable_by_key(|(_, modified, _)| *modified);
    Ok(matches
        .into_iter()
        .map(|(path, _, size)| (path, size))
        .collect())
}

fn read_use_percentage(directory: impl AsRef<Path>) -> Result<u8> {
    let stat = nix::sys::statvfs::statvfs(directory.as_ref())?;
    let total_blocks = stat.blocks();
    let free_blocks = stat.blocks_free();

    let use_percentage = u8::try_from(100u64 - (free_blocks * 100 / total_blocks))?;
    Ok(use_percentage)
}

fn compute_bytes_to_free(directory: impl AsRef<Path>, target_use_percentage: u8) -> Result<u64> {
    let stat = nix::sys::statvfs::statvfs(directory.as_ref())?;
    let total_blocks = stat.blocks();
    let free_blocks = stat.blocks_free();
    let used_blocks = total_blocks - free_blocks;
    let target_used_blocks = total_blocks * u64::from(target_use_percentage) / 100;

    if used_blocks <= target_used_blocks {
        return Ok(0);
    }

    let block_size = stat.fragment_size();
    Ok((used_blocks - target_used_blocks) * block_size)
}

fn dir_entry_to_modified(
    entry: walkdir::Result<walkdir::DirEntry>,
) -> Result<Option<(PathBuf, u64, u64)>> {
    let entry = entry?;
    if !entry.file_type().is_file() {
        return Ok(None);
    }

    let path = entry.path();
    let metadata = entry
        .metadata()
        .with_context(|| anyhow!("reading {:?}", &path))?;
    let modified = metadata
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let size = metadata.len();
    Ok(Some((path.to_path_buf(), modified, size)))
}
