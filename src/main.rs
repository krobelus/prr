use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::Parser;
use octocrab::Octocrab;
use serde_derive::Deserialize;

#[derive(Debug, Deserialize)]
struct PrrConfig {
    /// GH personal token
    token: String,
    /// Directory to place review files
    workdir: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Config {
    prr: PrrConfig,
}

#[derive(Parser, Debug)]
struct Args {
    /// Path to config file
    #[clap(long, parse(from_os_str))]
    config: Option<PathBuf>,
    /// Pull request to review (eg. `danobi/prr/24`)
    pr: String,
}

/// Main struct that coordinates all business logic and talks to GH
struct Prr {
    /// User config
    config: Config,
    /// Instantiated github client
    crab: Octocrab,
}

/// Represents the state of a single review
struct Review {
    /// Name of the owner of the repository
    owner: String,
    /// Name of the repository
    repo: String,
    /// Issue # of the pull request
    pr_num: u64,
}

impl Review {
    /// Creates a new `Review`
    ///
    /// `review_file` is the path where the user-facing review file should
    /// be created. Additional metadata files (dotfiles) may be created in the same
    /// directory.
    fn new(workdir: &Path, patch: String, owner: &str, repo: &str, pr_num: u64) -> Result<Review> {
        // First create directories leading up to review file if necessary
        let mut review_path = workdir.to_path_buf();
        review_path.push(owner);
        review_path.push(repo);
        fs::create_dir_all(&review_path).context("Failed to create workdir directories")?;

        // Now create review file
        review_path.push(pr_num.to_string());
        let mut review = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&review_path)
            .context("Failed to create review file")?;
        // XXX: prefix all patch lines with `>`
        review
            .write_all(patch.as_bytes())
            .context("Failed to write review file")?;

        // XXX: create metadata file (json) with orig contents and other info

        Ok(Review {
            owner: owner.to_owned(),
            repo: repo.to_owned(),
            pr_num,
        })
    }
}

impl Prr {
    fn new(config_path: &Path) -> Result<Prr> {
        let config_contents = fs::read_to_string(config_path).context("Failed to read config")?;
        let config: Config = toml::from_str(&config_contents).context("Failed to parse toml")?;
        let octocrab = Octocrab::builder()
            .personal_token(config.prr.token.clone())
            .build()
            .context("Failed to create GH client")?;

        Ok(Prr {
            config: config,
            crab: octocrab,
        })
    }

    fn workdir(&self) -> Result<PathBuf> {
        match &self.config.prr.workdir {
            Some(d) => {
                if d.starts_with("~") {
                    bail!("Workdir may not use '~' to denote home directory");
                }

                Ok(Path::new(d).to_path_buf())
            }
            None => {
                let xdg_dirs = xdg::BaseDirectories::with_prefix("prr")?;
                Ok(xdg_dirs.get_data_home())
            }
        }
    }

    async fn fetch_patch(&self, owner: &str, repo: &str, pr_num: u64) -> Result<Review> {
        let patch = self
            .crab
            .pulls(owner, repo)
            .get_patch(pr_num)
            .await
            .context("Failed to fetch patch file")?;

        Review::new(&self.workdir()?, patch, owner, repo, pr_num)
    }
}

/// Parses a PR string in the form of `danobi/prr/24` and returns
/// a tuple ("danobi", "prr", 24) or an error if string is malformed
fn parse_pr_str(s: &str) -> Result<(String, String, u64)> {
    let pieces: Vec<&str> = s.split('/').map(|ss| ss.trim()).collect();
    if pieces.len() != 3 {
        bail!("Invalid PR ref format: does not contain two '/'");
    }

    let owner = pieces[0].to_string();
    let repo = pieces[1].to_string();
    let pr_nr: u64 = pieces[2].parse()?;

    Ok((owner, repo, pr_nr))
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Figure out where config file is
    let config_path = match args.config {
        Some(c) => c,
        None => {
            let xdg_dirs = xdg::BaseDirectories::with_prefix("prr")?;
            xdg_dirs.get_config_file("config.toml")
        }
    };

    let prr = Prr::new(&config_path)?;
    let (owner, repo, pr_num) = parse_pr_str(&args.pr)?;
    let _ = prr.fetch_patch(&owner, &repo, pr_num).await?;

    Ok(())
}
