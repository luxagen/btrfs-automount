use anyhow::{Context, Result};
use clap::{Parser, ArgAction};
use regex::Regex;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use thiserror::Error;

#[derive(Error, Debug)]
enum BtramError {
    #[error("Missing or inaccessible config")]
    ConfigError,
    
    #[error("Bad config: {0}")]
    BadConfig(String),
    
    #[error("Bad subvol path: {0}")]
    BadSubvolPath(String),
    
    #[error("Command execution failed: {0}")]
    CommandFailed(String),
}

#[derive(Parser, Debug)]
#[clap(
    name = "btram",
    version = "1.0.0",
    author = "Luxagen <btram@luxagen.com>",
    about = "BTRFS auto-mounter companion tool to btrbk"
)]
struct Opts {
    /// Actually perform [re]mount operations
    #[clap(short = 'r', long = "run", action = ArgAction::SetTrue)]
    run: bool,
    
    /// Print analysis details
    #[clap(short = 'v', long = "verbose", action = ArgAction::SetTrue)]
    verbose: bool,
}

#[derive(Debug, Clone)]
struct ConfigEntry {
    root: String,
    dir: String,
    prefix: String,
    done: bool,
}

#[derive(Debug)]
struct MountInfo {
    blkdev: String,
    mount: String,
    fstype: String,
    options: HashMap<String, Option<String>>,
}

#[derive(Debug)]
struct SubvolInfo {
    flags: String,
    received_uuid: String,
    generation: String,
    gen_at_creation: String,
}

/// Main entry point for the btram application
pub fn run() -> Result<()> {
    let opts = Opts::parse();
    
    // Configure stdout/stderr to be line-buffered
    // (Rust handles this automatically)
    
    let config = get_config("/etc/btram.conf")?;
    
    // Map between device nodes and root-subvol mounts
    let mut bd2root: HashMap<String, String> = HashMap::new();
    let mut root2bd: HashMap<String, String> = HashMap::new();
    
    // Get current mounts
    let output = Command::new("mount")
        .stdout(Stdio::piped())
        .output()
        .context("Failed to execute mount command")?;
    
    let mut config_mut = config.clone();
    
    for line in BufReader::new(io::Cursor::new(output.stdout)).lines() {
        let line = line?;
        if let Some(m) = parse_mount(&line) {
            if m.fstype != "btrfs" {
                continue; // We don't care about non-btrfs mounts
            }
            
            let blkdev = m.blkdev.clone();
            let mount = m.mount.clone();
            
            if let Some(subvol) = m.options.get("subvol").and_then(|s| s.clone()) {
                if let Some(sv_new) = process_mount(
                    &blkdev, 
                    &mount, 
                    Some(&subvol), 
                    &mut bd2root, 
                    &mut root2bd, 
                    &mut config_mut, 
                    opts.verbose, 
                    opts.run
                )? {
                    mount_subvol(&mount, &blkdev, &sv_new, true, opts.verbose, opts.run)?;
                }
            } else {
                if let Some(sv_new) = process_mount(
                    &blkdev, 
                    &mount, 
                    None, 
                    &mut bd2root, 
                    &mut root2bd, 
                    &mut config_mut, 
                    opts.verbose, 
                    opts.run
                )? {
                    mount_subvol(&mount, &blkdev, &sv_new, true, opts.verbose, opts.run)?;
                }
            }
        }
    }
    
    // Handle remaining config entries
    for (mount, cfg) in config_mut.iter_mut() {
        if cfg.done {
            continue;
        }
        
        println!("Mount [fresh]: {}", mount);
        
        // We must have previously processed the root-subvol mount line for this block device
        let root_mount = &cfg.root;
        
        // Find the latest snapshot
        if let Some(sv_new) = find_latest(root_mount, &cfg.dir, &cfg.prefix)? {
            if let Some(blkdev) = root2bd.get(root_mount) {
                mount_subvol(mount, blkdev, &sv_new, false, opts.verbose, opts.run)?;
            } else {
                eprintln!("Warning: Could not find block device for root mount {}", root_mount);
            }
        } else {
            eprintln!("Warning: '{}' has no snapshots '{}/{}/{}.*'", 
                mount, root_mount, cfg.dir, cfg.prefix);
        }
    }
    
    Ok(())
}

fn get_config(file_path: &str) -> Result<HashMap<String, ConfigEntry>> {
    let mut result = HashMap::new();
    
    let file = File::open(file_path)
        .map_err(|_| BtramError::ConfigError)?;
    
    for line in BufReader::new(file).lines() {
        let line = line?;
        
        if line.trim().is_empty() {
            continue;
        }
        
        // Split by tabs (one or more)
        let parts: Vec<&str> = line.split('\t').filter(|s| !s.is_empty()).collect();
        
        if parts.len() != 4 {
            return Err(BtramError::BadConfig("Invalid number of columns".to_string()).into());
        }
        
        let mount = parts[0];
        let root = parts[1];
        let dir = parts[2];
        let prefix = parts[3];
        
        // Validate paths
        if !Path::new(mount).is_dir() {
            return Err(BtramError::BadConfig(format!("Mount point '{}' is not a directory", mount)).into());
        }
        
        if !Path::new(root).is_dir() {
            return Err(BtramError::BadConfig(format!("Root '{}' is not a directory", root)).into());
        }
        
        let snapshot_dir = format!("{}/{}", root, dir);
        if !Path::new(&snapshot_dir).is_dir() {
            return Err(BtramError::BadConfig(format!("Snapshot directory '{}' is not a directory", snapshot_dir)).into());
        }
        
        result.insert(mount.to_string(), ConfigEntry {
            root: root.to_string(),
            dir: dir.to_string(),
            prefix: prefix.to_string(),
            done: false,
        });
    }
    
    Ok(result)
}

fn parse_mount(line: &str) -> Option<MountInfo> {
    let re = Regex::new(r"^([^\s]+)\s+on\s+([^\s]+)\s+type\s+(.+)\s+\((.+)\)$").unwrap();
    
    if let Some(caps) = re.captures(line) {
        let blkdev = caps.get(1)?.as_str().to_string();
        let mount = caps.get(2)?.as_str().to_string();
        let fstype = caps.get(3)?.as_str().to_string();
        let options_str = caps.get(4)?.as_str();
        
        let mut options = HashMap::new();
        for opt in options_str.split(',') {
            if let Some((key, value)) = opt.split_once('=') {
                options.insert(key.to_string(), Some(value.to_string()));
            } else {
                options.insert(opt.to_string(), None);
            }
        }
        
        return Some(MountInfo {
            blkdev,
            mount,
            fstype,
            options,
        });
    }
    
    None
}

fn sv_info(root_mount: &str, sv: &str) -> Result<SubvolInfo> {
    let output = Command::new("btrfs")
        .args(["subvolume", "show", &format!("{}/{}", root_mount, sv)])
        .output()
        .context("Failed to execute btrfs subvolume show command")?;
    
    if !output.status.success() {
        return Err(BtramError::CommandFailed(format!("btrfs subvolume show failed for {}/{}", root_mount, sv)).into());
    }
    
    let output_str = String::from_utf8(output.stdout)?;
    let lines: Vec<&str> = output_str.lines().collect();
    
    if lines.is_empty() || lines[0] != sv {
        return Err(BtramError::CommandFailed(format!("Unexpected output from btrfs subvolume show")).into());
    }
    
    let mut flags = String::new();
    let mut received_uuid = String::new();
    let mut generation = String::new();
    let mut gen_at_creation = String::new();
    
    for line in lines.iter().skip(1) {
        if line.contains("Snapshot(s):") {
            break; // Stop when the snapshot list starts
        }
        
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            let value = value.trim();
            
            match key {
                "Flags" => flags = value.to_string(),
                "Received UUID" => received_uuid = value.to_string(),
                "Generation" => generation = value.to_string(),
                "Gen at creation" => gen_at_creation = value.to_string(),
                _ => {}
            }
        }
    }
    
    Ok(SubvolInfo {
        flags,
        received_uuid,
        generation,
        gen_at_creation,
    })
}

fn sv_impure(svi: &SubvolInfo) -> Option<&'static str> {
    if svi.flags != "readonly" {
        return Some("RW");
    }
    
    if svi.received_uuid != "-" {
        return None;
    }
    
    if svi.generation != svi.gen_at_creation {
        return Some("modified");
    }
    
    None
}

fn find_latest(root_mount: &str, ssdir: &str, ssn_prefix: &str) -> Result<Option<String>> {
    if ssn_prefix.is_empty() {
        return Err(BtramError::BadConfig("Empty snapshot prefix".to_string()).into());
    }
    
    let glob_pattern = format!("{}/{}/{}.*", root_mount, ssdir, ssn_prefix);
    let mut latest_ssn = None;
    
    for entry in glob::glob(&glob_pattern)? {
        let path = entry?;
        let file_name = path.file_name().unwrap().to_string_lossy();
        
        // Extract the timestamp part
        let re = Regex::new(&format!(r"^{}\.([\d_]+)$", regex::escape(ssn_prefix))).unwrap();
        if let Some(caps) = re.captures(&file_name) {
            let timestamp = caps.get(1).unwrap().as_str();
            let ssn_candidate = format!("{}.{}", ssn_prefix, timestamp);
            
            // Get subvolume info and check if it's impure
            let subvol_path = format!("{}/{}", ssdir, ssn_candidate);
            let sv_info = sv_info(root_mount, &subvol_path)?;
            
            if let Some(status) = sv_impure(&sv_info) {
                eprintln!("Warning: ignoring {} subvol '{}/{}'", status, ssdir, ssn_candidate);
                continue;
            }
            
            // Update latest if this is newer
            if latest_ssn.is_none() || ssn_candidate > latest_ssn.as_ref().unwrap() {
                latest_ssn = Some(ssn_candidate);
            }
        }
    }
    
    Ok(latest_ssn.map(|ssn| format!("{}/{}", ssdir, ssn)))
}

fn process_mount(
    blkdev: &str,
    mount: &str,
    sv: Option<&str>,
    bd2root: &mut HashMap<String, String>,
    root2bd: &mut HashMap<String, String>,
    config: &mut HashMap<String, ConfigEntry>,
    verbose: bool,
    run_mode: bool,
) -> Result<Option<String>> {
    if let Some(sv) = sv {
        // Special handling for root-subvol mounts
        if sv == "/" {
            bd2root.insert(blkdev.to_string(), mount.to_string());
            root2bd.insert(mount.to_string(), blkdev.to_string());
            return Ok(None);
        }
        
        // Strip leading slash from subvol path
        let sv = if sv.starts_with('/') {
            &sv[1..]
        } else {
            return Err(BtramError::BadSubvolPath(sv.to_string()).into());
        };
        
        // We must have previously processed the root-subvol mount line for this block device
        let root_mount = match bd2root.get(blkdev) {
            Some(rm) => rm,
            None => {
                eprintln!("Warning: {} has a subvol at '{}', but its root subvol is not mounted", blkdev, mount);
                return Ok(None);
            }
        };
        
        // Mark config entry as done if it exists
        if let Some(cfg) = config.get_mut(mount) {
            cfg.done = true;
        } else {
            eprintln!("Warning: mount '{}' is not mentioned in config", mount);
        }
        
        // Check if subvolume is impure
        let sv_info = sv_info(root_mount, sv)?;
        if let Some(status) = sv_impure(&sv_info) {
            if verbose {
                println!("Mount [skipped - {}]: '{}' -> {}/{}", status, mount, root_mount, sv);
            }
            return Ok(None);
        }
        
        // Snapshots live at least one directory deep
        let re = Regex::new(r"^(.+)/([^,\)]+)").unwrap();
        let caps = match re.captures(sv) {
            Some(c) => c,
            None => return Ok(None),
        };
        
        let ssdir = caps.get(1).unwrap().as_str();
        let ssn_curr = caps.get(2).unwrap().as_str();
        
        // Snapshots are named <prefix> . <number>
        let re = Regex::new(r"^(.+)\.\d+(?:_\d+)?").unwrap();
        let caps = match re.captures(ssn_curr) {
            Some(c) => c,
            None => return Ok(None),
        };
        
        let ss_prefix = caps.get(1).unwrap().as_str();
        
        if verbose {
            println!("Mount [candidate]: '{}' -> '{}/{}'", mount, root_mount, sv);
        }
        
        // Find the latest snapshot
        if let Some(sv_new) = find_latest(root_mount, ssdir, ss_prefix)? {
            if sv_new != sv {
                println!("'{}': '{}/{}' replaces '{}/{}'", mount, root_mount, sv_new, root_mount, sv);
                return Ok(Some(sv_new));
            }
        }
    } else {
        // No subvol specified
        eprintln!("Warning: mount '{}' has no subvol; was it deleted?", mount);
        
        if let Some(cfg) = config.get(mount) {
            let ssdir = &cfg.dir;
            let ss_prefix = &cfg.prefix;
            let root_mount = &cfg.root;
            
            if let Some(sv_new) = find_latest(root_mount, ssdir, ss_prefix)? {
                println!("'{}': {}/{} replaces <none>", mount, root_mount, sv_new);
                return Ok(Some(sv_new));
            }
        }
    }
    
    Ok(None)
}

fn mount_subvol(
    mount: &str,
    blkdev: &str,
    sv: &str,
    remount: bool,
    verbose: bool,
    run_mode: bool,
) -> Result<bool> {
    let umount_cmd = vec!["umount", mount];
    let mount_cmd = vec!["mount", "-o", &format!("ro,subvol={}", sv), blkdev, mount];
    
    if verbose {
        if remount {
            eprintln!("> {}", umount_cmd.join(" "));
        }
        eprintln!("> {}", mount_cmd.join(" "));
    }
    
    if !run_mode {
        return Ok(true);
    }
    
    if remount {
        let status = Command::new(umount_cmd[0])
            .args(&umount_cmd[1..])
            .status()
            .context("Failed to execute umount command")?;
            
        if !status.success() {
            eprintln!("Couldn't unmount '{}'", mount);
            return Ok(false);
        }
    }
    
    let status = Command::new(mount_cmd[0])
        .args(&mount_cmd[1..])
        .status()
        .context("Failed to execute mount command")?;
        
    if !status.success() {
        eprintln!("Couldn't mount '{}'!", mount);
        return Ok(false);
    }
    
    Ok(true)
}

#[cfg(test)]
mod tests;
