use crate::{
    CockroachDbConnectorTag, ConnectorTag, ConnectorVersion, MongoDbConnectorTag, MySqlConnectorTag,
    PostgresConnectorTag, SqlServerConnectorTag, SqliteConnectorTag, TestResult, VitessConnectorTag,
};
use serde::{Deserialize, Serialize};
use std::{convert::TryFrom, env, fmt::Display, fs::File, io::Read, path::PathBuf};

static TEST_CONFIG_FILE_NAME: &str = ".test_config";

#[derive(Debug, Deserialize, Clone)]
pub enum TestExecutor {
    Napi,
    Wasm,
}

impl Display for TestExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TestExecutor::Napi => f.write_str("Napi"),
            TestExecutor::Wasm => f.write_str("Wasm"),
        }
    }
}

/// The central test configuration.
#[derive(Debug, Default, Deserialize)]
pub struct TestConfig {
    /// The connector that tests should run for.
    /// Env key: `TEST_CONNECTOR`
    pub(crate) connector: String,

    /// The connector version tests should run for.
    /// If the test connector is versioned, this option is required.
    /// Env key: `TEST_CONNECTOR_VERSION`
    #[serde(rename = "version")]
    pub(crate) connector_version: Option<String>,

    /// An external process to execute the test queries and produced responses for assertion
    /// Used when testing driver adapters, this process is expected to be a javascript process
    /// loading the library engine (as a library, or WASM modules) and providing it with a
    /// driver adapter.
    /// Possible values: Napi, Wasm
    /// Env key: `EXTERNAL_TEST_EXECUTOR`
    pub(crate) external_test_executor: Option<TestExecutor>,

    /// The driver adapter to use when running tests, will be forwarded to the external test
    /// executor by setting the `DRIVER_ADAPTER` env var when spawning the executor process
    pub(crate) driver_adapter: Option<String>,

    /// The driver adapter configuration to forward as a stringified JSON object to the external
    /// test executor by setting the `DRIVER_ADAPTER_CONFIG` env var when spawning the executor
    pub(crate) driver_adapter_config: Option<DriverAdapterConfig>,

    /// Indicates whether or not the tests are running in CI context.
    /// Env key: `BUILDKITE`
    #[serde(default)]
    pub(crate) is_ci: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct DriverAdapterConfig {
    pub(crate) proxy_url: Option<String>,
}

const CONFIG_LOAD_FAILED: &str = r####"
=============================================
🔴 Unable to load config from file or env. 🔴
=============================================

ℹ️  How do I fix this? ℹ️ 

Test config can come from the environment, or a config file.

♻️  Environment variables

Be sure to have WORKSPACE_ROOT set to the root of the prisma-engines 
repository.

Set the following vars to denote the connector under test

- TEST_CONNECTOR
- TEST_CONNECTOR_VERSION (optional)

And optionally, to test driver adapters

- EXTERNAL_TEST_EXECUTOR
- DRIVER_ADAPTER
- DRIVER_ADAPTER_CONFIG (optional, not required by all driver adapters)

📁 Config file

Use the Makefile.
"####;

fn exit_with_message(msg: &str) -> ! {
    use std::io::{stderr, Write};
    let stderr = stderr();
    let mut sink = stderr.lock();
    sink.write_all(b"Error in the test configuration:\n").unwrap();
    sink.write_all(msg.as_bytes()).unwrap();
    sink.write_all(b"Aborting test process\n").unwrap();

    std::process::exit(1)
}

impl TestConfig {
    /// Loads a configuration. File-based config has precedence over env config.
    pub(crate) fn load() -> Self {
        let config = match Self::from_file().or_else(Self::from_env) {
            Some(config) => config,
            None => exit_with_message(CONFIG_LOAD_FAILED),
        };

        config.validate();
        config.log_info();

        config
    }

    #[rustfmt::skip]
    fn log_info(&self) {
        println!("******************************");
        println!("* Test run information:");
        println!(
            "* Connector: {} {}",
            self.connector,
            self.connector_version().unwrap_or_default()
        );
        println!("* CI? {}", self.is_ci);
        if let Some(external_test_executor) = self.external_test_executor.as_ref() {
            println!("* External test executor: {}", external_test_executor);
            println!("* Driver adapter: {}", self.driver_adapter().unwrap_or_default());
            println!("* Driver adapter config: {}", self.json_stringify_driver_adapter_config());
        }
        println!("******************************");
    }

    fn from_env() -> Option<Self> {
        let connector = std::env::var("TEST_CONNECTOR").ok();
        let connector_version = std::env::var("TEST_CONNECTOR_VERSION").ok();
        let external_test_executor = std::env::var("EXTERNAL_TEST_EXECUTOR")
            .map(|value| serde_json::from_str::<TestExecutor>(&value).ok())
            .unwrap_or_default();

        let driver_adapter = std::env::var("DRIVER_ADAPTER").ok();
        let driver_adapter_config = std::env::var("DRIVER_ADAPTER_CONFIG")
            .map(|config| serde_json::from_str::<DriverAdapterConfig>(config.as_str()).ok())
            .unwrap_or_default();

        // Just care for a set value for now.
        let is_ci = std::env::var("BUILDKITE").is_ok();

        connector.map(|connector| Self {
            connector,
            connector_version,
            is_ci,
            external_test_executor,
            driver_adapter,
            driver_adapter_config,
        })
    }

    fn from_file() -> Option<Self> {
        let current_dir = env::current_dir().ok();

        current_dir
            .and_then(|path| Self::try_path(config_path(path)))
            .or_else(|| Self::workspace_root().and_then(|path| Self::try_path(config_path(path))))
    }

    fn try_path(path: PathBuf) -> Option<Self> {
        File::open(path).ok().and_then(|mut f| {
            let mut config = String::new();

            f.read_to_string(&mut config)
                .ok()
                .and_then(|_| serde_json::from_str(&config).ok())
        })
    }

    fn workspace_root() -> Option<PathBuf> {
        env::var("WORKSPACE_ROOT").ok().map(PathBuf::from)
    }

    pub fn external_test_executor_path(&self) -> Option<String> {
        const DEFAULT_TEST_EXECUTOR: &str =
            "query-engine/driver-adapters/connector-test-kit-executor/script/start_node.sh";
        self.external_test_executor
            .as_ref()
            .and_then(|_| {
                Self::workspace_root().or_else(|| {
                    exit_with_message(
                        "WORKSPACE_ROOT needs to be correctly set to the root of the prisma-engines repository",
                    )
                })
            })
            .map(|path| path.join(DEFAULT_TEST_EXECUTOR))
            .and_then(|path| path.to_str().map(|s| s.to_owned()))
    }

    fn validate(&self) {
        if self.connector.is_empty() {
            exit_with_message("A test connector is required but was not set.");
        }

        match self.test_connector().map(|(_, v)| v) {
            Ok(ConnectorVersion::Vitess(None))
            | Ok(ConnectorVersion::MySql(None))
            | Ok(ConnectorVersion::SqlServer(None))
            | Ok(ConnectorVersion::MongoDb(None))
            | Ok(ConnectorVersion::CockroachDb(None))
            | Ok(ConnectorVersion::Postgres(None))
            | Ok(ConnectorVersion::Sqlite(None)) => {
                exit_with_message("The current test connector requires a version to be set to run.");
            }
            Ok(ConnectorVersion::Vitess(Some(_)))
            | Ok(ConnectorVersion::MySql(Some(_)))
            | Ok(ConnectorVersion::SqlServer(Some(_)))
            | Ok(ConnectorVersion::MongoDb(Some(_)))
            | Ok(ConnectorVersion::CockroachDb(Some(_)))
            | Ok(ConnectorVersion::Postgres(Some(_)))
            | Ok(ConnectorVersion::Sqlite(Some(_))) => (),
            Err(err) => exit_with_message(&err.to_string()),
        }

        if let Some(file) = self.external_test_executor_path().as_ref() {
            let path = PathBuf::from(file);
            let md = path.metadata();
            if !path.exists() || md.is_err() || !md.unwrap().is_file() {
                exit_with_message(&format!("The external test executor path `{}` must be a file", file));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let is_executable = match path.metadata() {
                    Err(_) => false,
                    Ok(md) => md.permissions().mode() & 0o111 != 0,
                };
                if !is_executable {
                    exit_with_message(&format!(
                        "The external test executor file `{}` must be have permissions to execute",
                        file
                    ));
                }
            }
        }

        if self.external_test_executor.is_some() && self.driver_adapter.is_none() {
            exit_with_message(
                "When using an external test executor, the driver adapter (DRIVER_ADAPTER env var) must be set.",
            );
        }

        if self.driver_adapter.is_some() && self.external_test_executor.is_none() {
            exit_with_message(
                "When using a driver adapter, the external test executor (EXTERNAL_TEST_EXECUTOR env var) must be set.",
            );
        }

        if self.driver_adapter.is_none() && self.driver_adapter_config.is_some() {
            exit_with_message(
                "When using a driver adapter config, the driver adapter (DRIVER_ADAPTER env var) must be set.",
            );
        }
    }

    pub fn connector(&self) -> &str {
        self.connector.as_str()
    }

    pub(crate) fn connector_version(&self) -> Option<&str> {
        self.connector_version.as_deref()
    }

    pub fn is_ci(&self) -> bool {
        self.is_ci
    }

    pub fn external_test_executor(&self) -> Option<TestExecutor> {
        self.external_test_executor.clone()
    }

    pub fn driver_adapter(&self) -> Option<&str> {
        self.driver_adapter.as_deref()
    }

    fn json_stringify_driver_adapter_config(&self) -> String {
        serde_json::to_string(&self.driver_adapter_config).unwrap_or_default()
    }

    pub fn test_connector(&self) -> TestResult<(ConnectorTag, ConnectorVersion)> {
        let version = ConnectorVersion::try_from((self.connector(), self.connector_version()))?;
        let tag = match version {
            ConnectorVersion::SqlServer(_) => &SqlServerConnectorTag as ConnectorTag,
            ConnectorVersion::Postgres(_) => &PostgresConnectorTag,
            ConnectorVersion::MySql(_) => &MySqlConnectorTag,
            ConnectorVersion::MongoDb(_) => &MongoDbConnectorTag,
            ConnectorVersion::Sqlite(_) => &SqliteConnectorTag,
            ConnectorVersion::CockroachDb(_) => &CockroachDbConnectorTag,
            ConnectorVersion::Vitess(_) => &VitessConnectorTag,
        };

        Ok((tag, version))
    }

    #[rustfmt::skip]
    pub fn for_external_executor(&self) -> Vec<(String, String)> {
        vec!(
            (
                "DRIVER_ADAPTER".to_string(), 
                self.driver_adapter.clone().unwrap_or_default()
            ),
            (
                "DRIVER_ADAPTER_CONFIG".to_string(),
                self.json_stringify_driver_adapter_config()
            ),
            (
                "EXTERNAL_TEST_EXECUTOR".to_string(),
                self.external_test_executor.clone().unwrap_or(TestExecutor::Napi).to_string(),
            ),
            (
                "PRISMA_DISABLE_QUAINT_EXECUTORS".to_string(),
                "1".to_string(),
            ),
        )
    }
}

fn config_path(mut path: PathBuf) -> PathBuf {
    path.push(TEST_CONFIG_FILE_NAME);
    path
}
