use std::{
    env,
    fs::{self, File},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
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

static API: &'static str = "https://api.opensuse.org";

#[macro_use]
extern crate lazy_static;

lazy_static! {
    static ref XDG: BaseDirectories = BaseDirectories::with_prefix("osutil")
        .context("unable to initialize XDG Base Directories")
        .unwrap();
    static ref CONFIG_FILE: PathBuf = XDG
        .place_config_file("osutil.conf")
        .context("unable to get config file")
        .unwrap();
    static ref CONFIG: Config = toml::from_str(
        &fs::read_to_string(&CONFIG_FILE.as_os_str())
            .with_context(|| format!("unable to read file {:?}", CONFIG_FILE.to_string_lossy()))
            .unwrap()
    )
    .with_context(|| format!(
        "unable to parse config file {:?}",
        CONFIG_FILE.to_string_lossy()
    ))
    .unwrap();
}

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
    #[clap(long = "leap")]
    leap_ver: Option<String>,
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
struct ObsSearchPackage {
    project: String,
    name: String,
}

#[derive(Deserialize)]
struct ObsSearchCollection {
    matches: String,
    #[serde(rename = "package")]
    packages: Vec<ObsSearchPackage>,
}

#[derive(Deserialize)]
struct ObsSourcePackage {
    project: String,
    package: String,
    target: ObsSourceTarget,
}

#[derive(Deserialize)]
struct ObsSourceTarget {
    project: String,
    package: String,
}

#[derive(Deserialize)]
struct ObsSourceCollection {
    #[serde(rename = "package")]
    packages: Vec<ObsSourcePackage>,
}

#[derive(Deserialize)]
struct Config {
    username: String,
    password: String,
}

async fn get_maintained_pkgs() -> Result<Vec<String>> {
    let client = Client::new();

    let text = client
        .get(format!(
            "{}/search/package/id?match=person/@userid+=+'{}'+and+person/@role+=+'maintainer'",
            API, CONFIG.username
        ))
        .header(
            "Authorization",
            format!(
                "Basic {}",
                base64::encode(format!("{}:{}", CONFIG.username, CONFIG.password))
            ),
        )
        .send()
        .await
        .context("unable to get maintained projects")?
        .text()
        .await
        .context("unable to get maintained projects")?;

    let collection: ObsSearchCollection = from_reader(text.as_bytes())?;
    Ok(collection
        .packages
        .iter()
        .map(|pkg| pkg.name.to_owned())
        .collect::<Vec<String>>())
}

async fn handle_pkg(
    (pkg, client, show_packages_not_found, leap_ver): (String, &Client, bool, &Option<String>),
) -> Result<()> {
    let repo_pkg = pkg.strip_prefix("python-").unwrap_or(&pkg);
    let repos = client
        .get(format!("https://repology.org/api/v1/project/{}", repo_pkg))
        .send()
        .await
        .with_context(|| {
            format!(
                "unable to get project information from repology for package {}{}",
                pkg,
                if repo_pkg != pkg {
                    format!(", searched for {}", repo_pkg)
                } else {
                    "".to_string()
                }
            )
        })?
        .json::<Vec<ProjectRepo>>()
        .await
        .with_context(|| format!("unable to deserialize json for package {}", pkg))?;
    let tw_repo = repos
        .iter()
        .find(|project_repo| project_repo.repo == "opensuse_tumbleweed");
    let newest_version = repos
        .iter()
        .find(|repo| repo.status == "newest")
        .map(|repo| repo.version.to_owned())
        .unwrap_or("?".to_string());
    if let Some(tw_repo) = tw_repo {
        if let Some(leap_ver) = leap_ver {
            let leap_repo = repos.iter().find(|project_repo| {
                project_repo.repo == format!("opensuse_leap_{}", leap_ver.replace('.', "_"))
            });
            if let Some(leap_repo) = leap_repo {
                if leap_repo.version != newest_version {
                    let text = client
                        .post(format!(
            "{}/source?cmd=branch&dryrun=1&package={}&update_project_attribute=OBS:UpdateProject",
            API, pkg
                        ))
                        .header(
                            "Authorization",
                            format!(
                                "Basic {}",
                                base64::encode(format!("{}:{}", CONFIG.username, CONFIG.password))
                            ),
                        )
                        .send()
                        .await
                        .context("unable to get maintained projects")?
                        .text()
                        .await
                        .unwrap();

                    let collection: ObsSourceCollection = from_reader(text.as_bytes())?;
                    let data = match leap_ver.as_str() {
                        "15.4" => ("SLE-15-SP4", vec!["SLE-15-SP3:Update", "SLE-15-SP2:Update"]),
                        _ => unimplemented!(),
                    };

                    let from_latest_sle = collection
                        .packages
                        .iter()
                        .find(|obs_package| obs_package.project == format!("SUSE:{}", data.0))
                        .is_some();
                    let from_latest_backports = collection
                        .packages
                        .iter()
                        .find(|obs_package| {
                            obs_package.project == format!("openSUSE:Backports:{}", data.0)
                        })
                        .is_some();
                    let from_older_backports = collection
                        .packages
                        .iter()
                        .find(|obs_package| {
                            data.1
                                .iter()
                                .find(|ver| {
                                    obs_package.project == format!("openSUSE:Backports:{}", ver)
                                })
                                .is_some()
                        })
                        .is_some();
                    if from_latest_backports || (!from_latest_sle && from_older_backports) {
                        println!("{}: {} -> {}", pkg, leap_repo.version, newest_version);
                    }
                }
            }
        } else {
            if tw_repo.status == "outdated" {
                println!("{}: {} -> {}", pkg, tw_repo.version, newest_version);
            }
        }
    } else {
        if show_packages_not_found {
            println!("Could not find package {}", pkg);
        }
    }

    Ok(())
}

async fn process_outdated(opts: Outdated) -> Result<()> {
    let client = Client::new();
    tokio_stream::iter(get_maintained_pkgs().await?)
        .map(|pkg| (pkg, &client, opts.show_packages_not_found, &opts.leap_ver))
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

    match opts.subcmd {
        SubCommand::Outdated(o) => process_outdated(o).await,
        SubCommand::RequiredMacros(r) => print_required_macro(r).await,
    }
}
