use crate::storage::{self, rocks::Storage};
use crate::util::*;
use crate::{config, constants::git::*};
use crate::{
    config::{fs, structures},
    error::AppError,
};
use anyhow::Result;
use git2::Repository;
use log::{debug, error, info, warn};
use std::collections::HashMap;
use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    process,
    process::exit,
    sync::atomic::AtomicBool,
    thread::JoinHandle,
    time::{Duration, Instant},
};
use std::{
    process::Command,
    sync::{atomic::Ordering, Arc, Mutex},
};

#[derive(Debug)]
pub struct ManagedRepo {
    config: config::structures::RepoConfig,
    uuid: String,
    cache_dir: PathBuf,
    storage: Arc<Storage>,
}

fn generate_uuid() -> String {
    use uuid::Uuid;
    Uuid::new_v4()
        .to_simple()
        .encode_lower(&mut Uuid::encode_buffer())
        .to_string()
}

impl<'a> ManagedRepo {
    pub fn new(config: &config::structures::RepoConfig) -> Result<ManagedRepo, AppError> {
        let underlying = ManagedRepo::repo_from_config(config);
        let mut git_config = underlying?.config().expect("Failed to read repo config.");

        match git_config.get_bool(ENABLED_CONFIG_KEY) {
            Ok(true) => true,
            _ => {
                eprintln!(
                    "Hint: Set Git config key '{}' to true in directory '{:?}' to enable the repo.",
                    ENABLED_CONFIG_KEY, &config.path
                );
                return Err(AppError::NotEnabled());
            }
        };

        let uuid = match git_config.get_string(REPO_UUID_CONFIG_KEY) {
            Ok(s) => s,
            _ => {
                let s = generate_uuid();
                eprintln!("Assigning UUID {}", &s);
                git_config
                    .set_str(REPO_UUID_CONFIG_KEY, &s)
                    .expect("Setting UUID in the repo's Git config failed");
                s.to_string()
            }
        };

        let cache_dir = config::fs::cache_dir().join(&uuid);
        let db_dir = cache_dir.join("db");
        std::fs::create_dir_all(&db_dir).expect("Failed to create ManagedRepo cache dir");
        let storage = Storage::new(&db_dir)?;

        let repo = ManagedRepo {
            config: config.clone(),
            uuid: uuid,
            cache_dir,
            storage: Arc::new(storage),
        };

        info!("Cache in {:?}", &repo.cache_dir());

        Ok(repo)
    }

    pub fn storage(&self) -> Arc<Storage> {
        self.storage.clone()
    }

    fn repo_from_config(
        config: &config::structures::RepoConfig,
    ) -> Result<git2::Repository, AppError> {
        Ok(git2::Repository::open(&config.path)?)
    }

    fn underlying(&self) -> Result<git2::Repository, AppError> {
        ManagedRepo::repo_from_config(&self.config)
    }

    pub fn get_name(&self) -> Result<String, AppError> {
        Ok(self
            .config
            .path
            .file_name()
            .expect("Could not discern name from path")
            .to_str()
            .expect("Path contains invalid unicode")
            .to_owned())
    }

    pub fn get_uuid(&self) -> String {
        self.uuid.to_owned()
    }

    pub fn work_dir(&self) -> Result<Option<String>, AppError> {
        Ok(self
            .underlying()?
            .workdir()
            .map(|p| p.to_string_lossy().into()))
    }

    pub fn info_dir(&self) -> PathBuf {
        self.underlying().unwrap().path().join("info")
    }

    pub fn authoritative(&self) -> Option<String> {
        let underlying = self.underlying().unwrap();
        println!("Repo path is {:?}", &underlying.path());
        let mut alternates_file = self.info_dir();
        alternates_file.push(Path::new("alternates"));
        std::fs::read_to_string(alternates_file).ok()
    }

    pub fn head(&self) -> Result<(String, String), AppError> {
        let underlying = self.underlying().unwrap();
        let head = &underlying.head().unwrap();
        let name = &head.name().unwrap();
        let id = underlying.revparse_single(&name)?.id().to_string();
        Ok((name.to_string(), id))
    }

    pub fn summary(&self) {
        eprintln!("Statuses:");
        let underlying = self.underlying().unwrap();
        for status in underlying.statuses(None).iter() {
            for item in status.iter() {
                if !item.status().is_ignored() {
                    eprintln!("  {:?} {:?}", item.status(), item.path().unwrap());
                }
            }
        }
    }

    pub fn cache_dir(&self) -> &Path {
        self.cache_dir.as_path()
    }

    pub fn run(&mut self) -> Result<(), AppError> {
        eprintln!("Started repo worker {}", self);
        Ok(())
    }

    // Import will bring all objects from Git's storage into the RocksDB storage.
    pub fn import(&self) -> Result<(), AppError> {
        use std::io::BufRead;

        const sleep_duration: Duration = Duration::from_millis(5);
        const progress_interval: Duration = Duration::from_secs(15);
        const import_concurrency: usize = 64;

        let work_dir = self.work_dir().unwrap().unwrap();
        info!("Importing repo {}", work_dir);

        let work_queue = Arc::new(crossbeam_queue::ArrayQueue::<git2::Oid>::new(
            import_concurrency * 10000,
        ));
        let mut handles = Vec::<JoinHandle<()>>::new();

        // Prevent workers from exiting prematurely because the list is not fully populated.
        let write_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let stall_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let running = Arc::new(AtomicBool::new(true));
        let populating = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let repo = self.underlying().unwrap();

        for thread_num in 0..import_concurrency {
            let thread_num = thread_num.clone();
            let work_queue_clone = work_queue.clone();
            let populating_clone = populating.clone();
            let write_count_clone = write_count.clone();
            let stall_count_clone = stall_count.clone();

            let storage_clone = self.storage();
            let repo_path = repo.path().to_owned();
            let handle = std::thread::spawn(move || {
                let thread_repo = Repository::open(&repo_path).expect("Could not open repo");
                let odb = thread_repo.odb().expect("Opening ODB failed");
                loop {
                    if let Some(oid) = work_queue_clone.pop() {
                        match odb.read(oid) {
                            Ok(obj) => {
                                let key = format!("o:{}", oid.to_string());
                                let hdr = format!("{} {}\0", obj.kind().str(), obj.len());
                                let mut val = Vec::<u8>::from(hdr.as_bytes());
                                val.extend(obj.data());
                                if let Err(e) = storage_clone.put_bytes(&key.as_bytes(), &val[..])
                                {
                                    error!(
                                        "Thread {} failed to store object {}: {}",
                                        &thread_num, &oid, &e
                                    );
                                    panic!("Failed to store object");
                                } else {
                                    write_count_clone.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                            Err(err) => {
                                // TODO: Clean up the DB?
                                panic!("Failed to locate object {}: {}", oid, err);
                            }
                        }
                    } else {
                        if populating_clone.load(Ordering::Relaxed) {
                            stall_count_clone.fetch_add(1, Ordering::Relaxed);
                            std::thread::sleep(sleep_duration);
                        } else {
                            break;
                        }
                    }
                }
            });

            handles.push(handle);
        }

        // Report progress
        let progress_handle = {
            let running_clone = running.clone();
            let write_count_clone = write_count.clone();
            let stall_count_clone = stall_count.clone();

            std::thread::spawn(move || {
                while running_clone.load(Ordering::Relaxed) {
                    std::thread::sleep(progress_interval);
                    info!(
                        "Imported {} objects ({} stalls)",
                        write_count_clone.load(Ordering::Relaxed),
                        stall_count_clone.load(Ordering::Relaxed)
                    );
                    stall_count_clone.store(0, Ordering::Relaxed)
                }
            })
        };

        repo.odb().unwrap().foreach(|oid| {
            work_queue.push(oid.clone()).expect("push failed");
            true
        });

        // Tell workers that if the queue is empty, it's over.
        populating.store(false, std::sync::atomic::Ordering::SeqCst);

        for handle in handles {
            handle.join();
        }
        running.store(false, Ordering::Relaxed);
        progress_handle.join();
        info!("Finished importing repo {}", &work_dir);

        Ok(())
    }
}

impl std::fmt::Display for ManagedRepo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "uuid:{}, path:{}",
            self.uuid,
            self.config.path.to_str().unwrap()
        )
    }
}

type RepoMap = HashMap<String, Arc<Mutex<ManagedRepo>>>;

pub struct Repos {
    pub config: structures::Config,
    pub underlying: Mutex<RepoMap>,
}

impl Repos {
    pub fn new(config: Option<structures::Config>) -> Result<Self, AppError> {
        Self::ensure_dirs()?;
        let config = config.unwrap_or(Self::default_config());
        let mut repo_configs = Vec::<structures::RepoConfig>::new();

        if let Some(repos) = config.managed_repos.clone() {
            repo_configs.extend(repos);
        } else {
            let default_authoritative_repos = Self::default_managed_repos();
            repo_configs.extend(default_authoritative_repos);
        }
        let mut repos = RepoMap::new();
        repo_configs
            .iter()
            .for_each(|repo_config| match ManagedRepo::new(&repo_config) {
                Ok(repo) => {
                    let uuid = repo.get_uuid();
                    repos.insert(uuid, Arc::new(Mutex::new(repo)));
                }
                Err(e) => {
                    error!("Skipping repo {:?}: {}", repo_config, e);
                }
            });

        Ok(Self {
            config: config,
            underlying: Mutex::new(repos),
        })
    }

    pub fn shutdown(&self) -> Result<(), AppError> {
        let mut locked_repos = self
            .underlying
            .lock()
            .map_err(|_| AppError::WriteLockFailed())?;
        locked_repos.clear();
        info!("Shut down cleanly");
        return Ok(());
    }

    fn ensure_dirs() -> Result<(), AppError> {
        std::fs::create_dir_all(config::fs::config_dir()).map_err(|e| AppError::Io(e))?;
        std::fs::create_dir_all(config::fs::cache_dir()).map_err(|e| AppError::Io(e))?;
        Ok(())
    }

    fn enabled_repo_predicate(path: &Path) -> bool {
        if !path.is_dir() {
            return false;
        }

        match git2::Repository::open(path) {
            Ok(repo) => match repo.config() {
                Ok(config) => config.get_bool(ENABLED_CONFIG_KEY).unwrap_or(false),
                _ => false,
            },
            _ => false,
        }
    }

    fn default_managed_repos() -> Vec<structures::RepoConfig> {
        let mut configs = Vec::<structures::RepoConfig>::new();
        let repo_dir = fs::workspace_dir();
        info!("Discovering repositories in {:?}", &repo_dir);
        if let Ok(entries) = std::fs::read_dir(&repo_dir) {
            for entry in entries {
                if let Ok(entry) = entry {
                    if Self::enabled_repo_predicate(&entry.path()) {
                        info!("{:?} enabled", &entry.path());
                        configs.push(structures::RepoConfig {
                            path: entry.path().clone(),
                        })
                    } else {
                        info!("{:?} not enabled", &entry.path());
                    }

                }
            }
        };

        configs
    }

    fn default_config() -> config::structures::Config {
        let config_file_path = config::fs::config_dir().join(Path::new("focus.toml"));
        info!("Loading config from {:?}", config_file_path);
        let config_file_contents = std::fs::read_to_string(&config_file_path)
            .expect(format!("Config read error on {:?}", &config_file_path).as_str());
        toml::from_str(&config_file_contents).expect("Config parse error")
    }
}
