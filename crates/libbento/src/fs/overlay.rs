use anyhow::{Context, Result};
use nix::{
    mount::{mount, umount, MsFlags},
    //sys::stat::{Mode, SFlag},
};
use std::{
    ffi::CString,
    fs::DirBuilder,
    os::unix::fs::{DirBuilderExt},
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct OverlayPaths {
    pub lower_dir: PathBuf,
    pub upper_dir: PathBuf,
    pub work_dir: PathBuf,
    pub merged_dir: PathBuf,
}


pub fn build_overlayfs(user_id : u32, container_id : &str) -> Result<OverlayPaths> {
      let paths = init_overlay_paths(user_id, container_id);
      create_overlay_dirs(&paths).context("Failed to create directories")?;
      mount_overlay(&paths).context("Failed to mount directories")?;
 //     create_whiteout(&paths.upper_dir).context("Failed to create whiteouts");
 //     create_opaque_dir(&paths.upper_dir).context("Failed to create opaque directory");
      
      Ok(paths)
}


pub fn clear_overlayfs(paths : OverlayPaths) -> Result<()> {
      
      unmount_overlay(&paths.merged_dir).context("Failed to unmount directories")?;

      Ok(())
}

// Initializing overlay paths 
pub fn init_overlay_paths(user_id: u32, container_id: &str) -> OverlayPaths {
    
    let base_path = PathBuf::from(format!("/run/user/{}/container/{}", user_id, container_id));

    OverlayPaths {
        lower_dir: base_path.join("overlay/lower"),
        upper_dir: base_path.join("overlay/upper"),
        work_dir: base_path.join("overlay/work"),
        merged_dir: base_path.join("overlay/merged"),
    }
}

// Create all required directories with proper permissions
pub fn create_overlay_dirs(paths: &OverlayPaths) -> Result<()> {
   /* if !paths.lower_dir.exists() {
        return Err(anyhow::anyhow!("Lower directory does not exist: {:?}", paths.lower_dir));
    }
*/

    DirBuilder::new()
        .mode(0o755)
        .recursive(true)
        .create(&paths.lower_dir)
        .context("Failed to create lower dir")?;
    
    DirBuilder::new()
        .mode(0o755)
        .recursive(true)
        .create(&paths.upper_dir)
        .context("Failed to create upper dir")?;

    DirBuilder::new()
        .mode(0o755)
        .recursive(true)
        .create(&paths.work_dir)
        .context("Failed to create work dir")?;

    DirBuilder::new()
        .mode(0o755)
        .recursive(true)
        .create(&paths.merged_dir)
        .context("Failed to create merged dir")?;

    Ok(())
}

// Mount the overlay filesystem
pub fn mount_overlay(paths: &OverlayPaths) -> Result<()> {
    let options = format!(
        "lowerdir={},upperdir={},workdir={}",
        paths.lower_dir.to_str().context("Invalid lower_dir path")?,
        paths.upper_dir.to_str().context("Invalid upper_dir path")?,
        paths.work_dir.to_str().context("Invalid work_dir path")?
    );

    let fstype = CString::new("overlay")?;
    let target = CString::new(paths.merged_dir.to_str().context("Invalid merged_dir path")?)?;
    let data = CString::new(options)?;
   
    mount(
        Some(fstype.as_c_str()),
        target.as_c_str(),
        Some(fstype.as_c_str()),
        MsFlags::empty(),
        Some(data.as_c_str()),
    ).context("Failed to mount overlayfs")?;

    Ok(())
}


/*
// Creating a whiteout file
pub fn create_whiteout(upper_dir: &Path, path: &Path) -> Result<()> {
    let whiteout_path = upper_dir.join(path);
    let file_name = whiteout_path.file_name()
        .context("Path has no file name")?
        .to_str()
        .context("Invalid file name encoding")?;
    
    let whiteout_file = whiteout_path.with_file_name(format!(".wh.{}", file_name));

    if let Some(parent) = whiteout_file.parent() {
        fs::create_dir_all(parent).context("Failed to create parent directories")?;
    }

    if whiteout_file.exists() {
        return Err(anyhow::anyhow!("Whiteout file already exists: {:?}", whiteout_file));
    }

    mknod(
        &whiteout_file,
        SFlag::S_IFCHR,
        Mode::from_bits(0o644).unwrap(),
        0,
    ).context("Failed to create whiteout file")
}

// Creating an opaque directory marker
pub fn create_opaque_dir(upper_dir: &Path, path: &Path) -> Result<()> {
    let opaque_dir = upper_dir.join(path);
    fs::create_dir_all(&opaque_dir).context("Failed to create opaque dir parent")?;
    
    let opaque_path = opaque_dir.join(".wh..wh..opq");
    if opaque_path.exists() {
        return Err(anyhow::anyhow!("Opaque marker already exists: {:?}", opaque_path));
    }

    File::create(&opaque_path).context("Failed to create opaque dir marker")
}


*/

// Unmounting the overlay filesystem
pub fn unmount_overlay(merged_dir: &Path) -> Result<()> {
    umount(merged_dir).context("Failed to unmount overlayfs")?;
    Ok(())
}


