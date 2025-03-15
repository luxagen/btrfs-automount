#[cfg(test)]
mod tests {
    use crate::*;
    use std::fs;
    use std::io::Write;
    use tempfile::tempdir;
    
    #[test]
    fn test_parse_mount() {
        let line = "/dev/sda1 on /mnt type btrfs (rw,relatime,space_cache,subvolid=5,subvol=/)";
        let mount_info = parse_mount(line).unwrap();
        
        assert_eq!(mount_info.blkdev, "/dev/sda1");
        assert_eq!(mount_info.mount, "/mnt");
        assert_eq!(mount_info.fstype, "btrfs");
        assert!(mount_info.options.contains_key("rw"));
        assert!(mount_info.options.contains_key("relatime"));
        assert!(mount_info.options.contains_key("space_cache"));
        assert_eq!(mount_info.options.get("subvolid").unwrap().as_ref().unwrap(), "5");
        assert_eq!(mount_info.options.get("subvol").unwrap().as_ref().unwrap(), "/");
    }
    
    #[test]
    fn test_get_config() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("btram.conf");
        
        // Create test directories
        let mount_point = dir.path().join("mount");
        let root = dir.path().join("root");
        let snapshot_dir = root.join("snapshots");
        
        fs::create_dir(&mount_point).unwrap();
        fs::create_dir(&root).unwrap();
        fs::create_dir(&snapshot_dir).unwrap();
        
        // Create test config file
        let config_content = format!(
            "{}\t{}\t{}\t{}\n",
            mount_point.to_str().unwrap(),
            root.to_str().unwrap(),
            "snapshots",
            "snap"
        );
        
        let mut file = File::create(&config_path).unwrap();
        file.write_all(config_content.as_bytes()).unwrap();
        
        // Test config parsing
        let config = get_config(config_path.to_str().unwrap()).unwrap();
        
        assert_eq!(config.len(), 1);
        let entry = config.get(mount_point.to_str().unwrap()).unwrap();
        assert_eq!(entry.root, root.to_str().unwrap());
        assert_eq!(entry.dir, "snapshots");
        assert_eq!(entry.prefix, "snap");
        assert_eq!(entry.done, false);
    }
    
    #[test]
    fn test_find_latest() {
        // This test would require mocking the btrfs command output
        // For now, we'll just test the regex pattern
        let re = Regex::new(r"^(.+)\.([\d_]+)$").unwrap();
        
        assert!(re.is_match("snap.20230101"));
        assert!(re.is_match("snap.20230101_1234"));
        
        let caps = re.captures("snap.20230101").unwrap();
        assert_eq!(caps.get(1).unwrap().as_str(), "snap");
        assert_eq!(caps.get(2).unwrap().as_str(), "20230101");
    }
    
    #[test]
    fn test_sv_impure() {
        // Test readonly, clean snapshot
        let clean = SubvolInfo {
            flags: "readonly".to_string(),
            received_uuid: "-".to_string(),
            generation: "100".to_string(),
            gen_at_creation: "100".to_string(),
        };
        assert_eq!(sv_impure(&clean), None);
        
        // Test read-write snapshot
        let rw = SubvolInfo {
            flags: "".to_string(),
            received_uuid: "-".to_string(),
            generation: "100".to_string(),
            gen_at_creation: "100".to_string(),
        };
        assert_eq!(sv_impure(&rw), Some("RW"));
        
        // Test modified snapshot
        let modified = SubvolInfo {
            flags: "readonly".to_string(),
            received_uuid: "-".to_string(),
            generation: "101".to_string(),
            gen_at_creation: "100".to_string(),
        };
        assert_eq!(sv_impure(&modified), Some("modified"));
    }
}
