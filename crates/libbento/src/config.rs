use anyhow::Result;

#[derive(Debug)]
pub struct Config {
    pub root: RootConfig,
}

#[derive(Debug)]
pub struct RootConfig {
    pub path: String,
}

pub fn load_config(container_id: &str) -> Result<Config> {
    let root_path = format!("/run/container/{container_id}/rootfs");
    Ok(Config {
        root: RootConfig { path: root_path },
    })
}
