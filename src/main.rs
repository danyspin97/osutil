use std::{
    env,
    fs::{self, File},
    io::{BufRead, BufReader},
    path::Path,
};

use clap::Parser;
use color_eyre::eyre::{Context, ContextCompat, Result};
use futures::StreamExt;
use regex::Regex;
use reqwest::{self, Client};
use serde::Deserialize;
use serde_xml_rs::from_reader;
use toml;
use xdg::BaseDirectories;

#[derive(Parser)]
#[clap(version = "0.1", author = "Danilo Spinella <danilo.spinella@suse.com>")]
struct Opts {
    #[clap(subcommand)]
    subcmd: SubCommand,
}

#[derive(Parser)]
enum SubCommand {
    #[clap()]
    Outdated(Outdated),
    #[clap()]
    RequiredMacros(RequiredMacros),
}

#[derive(Parser)]
struct Outdated {
    #[clap(short = 'n', long)]
    show_packages_not_found: bool,
}

#[derive(Parser)]
struct RequiredMacros {}

#[derive(Deserialize)]
struct ProjectRepo {
    repo: String,
    subrepo: Option<String>,
    srcname: Option<String>,
    visiblename: String,
    version: String,
    maintainers: Option<Vec<String>>,
    categories: Option<Vec<String>>,
    status: String,
    origversion: Option<String>,
}

#[derive(Deserialize)]
struct ObsPackage {
    project: String,
    name: String,
}

#[derive(Deserialize)]
struct ObsCollection {
    matches: String,
    #[serde(rename = "package")]
    packages: Vec<ObsPackage>,
}

#[derive(Deserialize)]
struct Config {
    username: String,
    password: String,
}

async fn get_maintained_pkgs(config: Config) -> Result<Vec<String>> {
    let client = Client::new();

    let api = "https://api.opensuse.org";
    let text = client
        .get(format!(
            "{}/search/package/id?match=person/@userid+=+'{}'+and+person/@role+=+'maintainer'",
            api, config.username
        ))
        .header(
            "Authorization",
            format!(
                "Basic {}",
                base64::encode(format!("{}:{}", config.username, config.password))
            ),
        )
        .send()
        .await
        .context("unable to get maintained projects")?
        .text()
        .await
        .context("unable to get maintained projects")?;

    let collection: ObsCollection = from_reader(text.as_bytes())?;
    Ok(collection
        .packages
        .iter()
        .map(|pkg| pkg.name.to_owned())
        .collect::<Vec<String>>())
}

async fn handle_pkg((pkg, client, show_packages_not_found): (String, &Client, bool)) -> Result<()> {
    let repos = client
        .get(format!("https://repology.org/api/v1/project/{}", pkg))
        .send()
        .await
        .with_context(|| {
            format!(
                "unable to get project information from repology for package {}",
                pkg
            )
        })?
        .json::<Vec<ProjectRepo>>()
        .await
        .with_context(|| format!("unable to deserialize json for package {}", pkg))?;
    let tw_repo = repos
        .iter()
        .find(|project_repo| project_repo.repo == "opensuse_tumbleweed");
    if let Some(tw_repo) = tw_repo {
        if tw_repo.status == "outdated" {
            let newest_version = repos
                .iter()
                .find(|repo| repo.status == "newest")
                .map(|repo| repo.version.to_owned())
                .unwrap_or("?".to_string());
            println!("{}: {} -> {}", pkg, tw_repo.version, newest_version);
        }
    } else {
        if show_packages_not_found {
            println!("Could not find package {}", pkg);
        }
    }

    Ok(())
}

async fn process_outdated(opts: Outdated, config: Config) -> Result<()> {
    let client = Client::new();
    tokio_stream::iter(get_maintained_pkgs(config).await?)
        .map(|pkg| (pkg, &client, opts.show_packages_not_found))
        .map(handle_pkg)
        .buffer_unordered(4)
        .for_each(|res| async {
            match res {
                Ok(_) => {}
                Err(err) => eprintln!("{}", err),
            }
        })
        .await;
    Ok(())
}

fn get_pkg_name() -> Result<String> {
    let path = env::current_dir()
        .context("unable to get current directory")?
        .as_os_str()
        .to_str()
        .context("unable to convert to string")?
        .to_owned();
    // all the paths are ./pkg.SLE_VERSION
    // skip the starting prefix ./
    let mut split = Path::new(&path)
        .file_name()
        .context("unable to get directory name")?
        .to_str()
        .context("unable to get directory name")?
        .split(".");

    Ok(split.next().context("invalid directory name")?.to_string())
}

async fn print_required_macro(_: RequiredMacros) -> Result<()> {
    let re = Regex::new(
        r"BuildRequires: *((pkgconfig|user)\((?P<pkg>\w+(-\w*)*)\)|(?P<pkg2>\w.*(-\w.*))) *(((>?=)|<) .* .*)?",
    )
    .context("invalid regex")?;

    let pkg_name = get_pkg_name().context("unable to get pkg name")?;
    let spec_file = format!("{}.spec", pkg_name);

    print!(
        "{}",
        BufReader::new(
            File::open(&spec_file)
                .with_context(|| format!("unable to open spec file {}", spec_file))?,
        )
        .lines()
        .filter_map(|line| line.ok())
        .filter_map(|line| -> Option<String> {
            if let Some(cap) = re.captures(&line) {
                if let Some(pkg) = cap.name("pkg") {
                    Some(pkg.as_str().to_owned())
                } else if let Some(pkg) = cap.name("pkg2") {
                    Some(pkg.as_str().to_owned())
                } else {
                    None
                }
            } else {
                None
            }
        })
        .filter(|pkg| pkg.contains("macro") || pkg.contains("rpm"))
        .collect::<Vec<String>>()
        .join(" ")
    );

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let opts: Opts = Opts::parse();

    let xdg = BaseDirectories::with_prefix("osutil")
        .context("unable to initialize XDG Base Directories")?;
    let config_file = xdg
        .place_config_file("osutil.conf")
        .context("unable to get config file")?;
    let config = toml::from_str(
        &fs::read_to_string(&config_file)
            .with_context(|| format!("unable to read file {:?}", config_file))?,
    )
    .with_context(|| format!("unable to parse config file {:?}", config_file))?;

    match opts.subcmd {
        SubCommand::Outdated(o) => process_outdated(o, config).await,
        SubCommand::RequiredMacros(r) => print_required_macro(r).await,
    }
}
