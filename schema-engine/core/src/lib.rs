#![deny(rust_2018_idioms, unsafe_code, missing_docs)]
#![allow(clippy::needless_collect)] // the implementation of that rule is way too eager, it rejects necessary collects
#![allow(clippy::derive_partial_eq_without_eq)]

//! The top-level library crate for the schema engine.

include!(concat!(env!("OUT_DIR"), "/methods.rs"));

// exposed for tests
#[doc(hidden)]
pub mod commands;

mod api;
mod core_error;
mod rpc;
mod state;
mod timings;

pub use self::{api::GenericApi, core_error::*, rpc::rpc_api, timings::TimingsLayer};
pub use schema_connector;

use enumflags2::BitFlags;
use mongodb_schema_connector::MongoDbSchemaConnector;
use psl::{
    builtin_connectors::*, datamodel_connector::Flavour, parser_database::SourceFile, Datasource, PreviewFeature,
    ValidatedSchema,
};
use schema_connector::ConnectorParams;
use sql_schema_connector::SqlSchemaConnector;
use std::{env, path::Path};
use user_facing_errors::common::InvalidConnectionString;

fn parse_schema(schema: SourceFile) -> CoreResult<ValidatedSchema> {
    psl::parse_schema(schema).map_err(CoreError::new_schema_parser_error)
}

fn connector_for_connection_string(
    connection_string: String,
    shadow_database_connection_string: Option<String>,
    preview_features: BitFlags<PreviewFeature>,
) -> CoreResult<Box<dyn schema_connector::SchemaConnector>> {
    match connection_string.split(':').next() {
        Some("postgres") | Some("postgresql") => {
            let params = ConnectorParams {
                connection_string,
                preview_features,
                shadow_database_connection_string,
            };
            let mut connector = SqlSchemaConnector::new_postgres_like();
            connector.set_params(params)?;
            Ok(Box::new(connector))
        }
        Some("file") => {
            let params = ConnectorParams {
                connection_string,
                preview_features,
                shadow_database_connection_string,
            };
            let mut connector = SqlSchemaConnector::new_sqlite();
            connector.set_params(params)?;
            Ok(Box::new(connector))
        }
        Some("mysql") => {
            let params = ConnectorParams {
                connection_string,
                preview_features,
                shadow_database_connection_string,
            };
            let mut connector = SqlSchemaConnector::new_mysql();
            connector.set_params(params)?;
            Ok(Box::new(connector))
        }
        Some("sqlserver") => {
            let params = ConnectorParams {
                connection_string,
                preview_features,
                shadow_database_connection_string,
            };
            let mut connector = SqlSchemaConnector::new_mssql();
            connector.set_params(params)?;
            Ok(Box::new(connector))
        }
        Some("mongodb+srv") | Some("mongodb") => {
            let params = ConnectorParams {
                connection_string,
                preview_features,
                shadow_database_connection_string,
            };
            let connector = MongoDbSchemaConnector::new(params);
            Ok(Box::new(connector))
        }
        Some(_other) => Err(CoreError::url_parse_error("The scheme is not recognized")),
        None => Err(CoreError::user_facing(InvalidConnectionString {
            details: String::new(),
        })),
    }
}

/// Same as schema_to_connector, but it will only read the provider, not the connector params.
fn schema_to_connector_unchecked(schema: &str) -> CoreResult<Box<dyn schema_connector::SchemaConnector>> {
    let config = psl::parse_configuration(schema)
        .map_err(|err| CoreError::new_schema_parser_error(err.to_pretty_string("schema.prisma", schema)))?;

    let preview_features = config.preview_features();
    let source = config
        .datasources
        .into_iter()
        .next()
        .ok_or_else(|| CoreError::from_msg("There is no datasource in the schema.".into()))?;

    let mut connector = connector_for_provider(source.active_provider)?;

    if let Ok(connection_string) = source.load_direct_url(|key| env::var(key).ok()) {
        connector.set_params(ConnectorParams {
            connection_string,
            preview_features,
            shadow_database_connection_string: source.load_shadow_database_url().ok().flatten(),
        })?;
    }

    Ok(connector)
}

/// Go from a schema to a connector
fn schema_to_connector(
    schema: &str,
    config_dir: Option<&Path>,
) -> CoreResult<Box<dyn schema_connector::SchemaConnector>> {
    let (source, url, preview_features, shadow_database_url) = parse_configuration(schema)?;

    let url = config_dir
        .map(|config_dir| source.active_connector.set_config_dir(config_dir, &url).into_owned())
        .unwrap_or(url);

    let params = ConnectorParams {
        connection_string: url,
        preview_features,
        shadow_database_connection_string: shadow_database_url,
    };

    let mut connector = connector_for_provider(source.active_provider)?;
    connector.set_params(params)?;
    Ok(connector)
}

fn connector_for_provider(provider: &str) -> CoreResult<Box<dyn schema_connector::SchemaConnector>> {
    if let Some(connector) = BUILTIN_CONNECTORS.iter().find(|c| c.is_provider(provider)) {
        match connector.flavour() {
            Flavour::Cockroach => Ok(Box::new(SqlSchemaConnector::new_cockroach())),
            Flavour::Mongo => Ok(Box::new(MongoDbSchemaConnector::new(ConnectorParams {
                connection_string: String::new(),
                preview_features: Default::default(),
                shadow_database_connection_string: None,
            }))),
            Flavour::Sqlserver => Ok(Box::new(SqlSchemaConnector::new_mssql())),
            Flavour::Mysql => Ok(Box::new(SqlSchemaConnector::new_mysql())),
            Flavour::Postgres => Ok(Box::new(SqlSchemaConnector::new_postgres())),
            Flavour::Sqlite => Ok(Box::new(SqlSchemaConnector::new_sqlite())),
        }
    } else {
        Err(CoreError::from_msg(format!(
            "`{provider}` is not a supported connector."
        )))
    }
}

/// Top-level constructor for the schema engine API.
pub fn schema_api(
    datamodel: Option<String>,
    host: Option<std::sync::Arc<dyn schema_connector::ConnectorHost>>,
) -> CoreResult<Box<dyn api::GenericApi>> {
    // Eagerly load the default schema, for validation errors.
    if let Some(datamodel) = &datamodel {
        parse_configuration(datamodel)?;
    }

    let state = state::EngineState::new(datamodel, host);
    Ok(Box::new(state))
}

fn parse_configuration(datamodel: &str) -> CoreResult<(Datasource, String, BitFlags<PreviewFeature>, Option<String>)> {
    let config = psl::parse_configuration(datamodel)
        .map_err(|err| CoreError::new_schema_parser_error(err.to_pretty_string("schema.prisma", datamodel)))?;

    let preview_features = config.preview_features();

    let source = config
        .datasources
        .into_iter()
        .next()
        .ok_or_else(|| CoreError::from_msg("There is no datasource in the schema.".into()))?;

    let url = source
        .load_direct_url(|key| env::var(key).ok())
        .map_err(|err| CoreError::new_schema_parser_error(err.to_pretty_string("schema.prisma", datamodel)))?;

    let shadow_database_url = source
        .load_shadow_database_url()
        .map_err(|err| CoreError::new_schema_parser_error(err.to_pretty_string("schema.prisma", datamodel)))?;

    Ok((source, url, preview_features, shadow_database_url))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json_rpc::types::*;
    use std::fmt::Debug;
    use tracing_subscriber;
    #[derive(Debug)]
    struct DBConf {
        provider: String,
        url: String,
    }
    impl DBConf {
        fn to_string(&self) -> String {
            format!(
                "datasource db {{\n  provider = \"{}\"\n  url      = \"{}\"\n}}\n",
                self.provider, self.url
            )
        }
    }
    struct Case {
        name: String,
        schema: String,
    }

    const NEW_TABLE: &str = r##"
/// table comment
model User {
  /// id comment
  id      Int     @id @default(autoincrement())
  /// email comment
  email   Int     @unique
  /// name comment
  name    String?
  /// zjl
  comment String
  /// comment dd
  dd      String
}
            "##;
    const UPDATE_TABLE_COMMENT: &str = r##"
/// comment
model User {
  /// id comment
  id      Int     @id @default(autoincrement())
  /// email comment
  email   Int     @unique
  /// name comment
  name    String?
  /// zjl
  comment String
  /// comment dd
  dd      String
}
            "##;
    const UPDATE_TABLE_COMMENT_AND_COL: &str = r##"
/// use comment
model User {
  /// id comment
  id      Int     @id 
  /// email comment
  email   Int     @unique
  /// name commen
  name    String?
  /// comment
  comment String
}
            "##;
    const UPDATE_COMMENT: &str = r##"
/// use comment
model User {
  /// id comment
  id      Int     @id 
  /// email comment
  email   Int     @unique
  /// name comment
  name    String?
  /// comment
  comment String
}
            "##;
    const UPDATE_COL: &str = r##"
/// table comment
model User {
  /// id 
  id      Int     @id @default(autoincrement())
  /// email 
  email   Int     
  /// name 
  name    String
  /// comment
  comment Int
  /// comment dd
  dd      Int
}
            "##;
    const DEL_COL: &str = r##"
/// table comment
model User {
  /// id 
  id      Int     @id @default(autoincrement())
  /// email 
  email   Int
  /// comment
  comment Int
  /// comment ee
  ee String?
}
            "##;
    #[tokio::test]
    async fn test_all() {
        println!("先删除数据库的表");
        tracing_subscriber::fmt().with_max_level(tracing::Level::DEBUG).init();
        let ds = vec![
            DBConf {
                provider: "mysql".to_string(),
                url: "mysql://root:*@localhost:3306/zjl".to_string(),
            },
            DBConf {
                provider: "postgres".to_string(),
                url: "postgres://postgres:*@localhost:5432/zjl".to_string(),
            },
            DBConf {
                provider:"sqlserver".to_string(),
                url:"sqlserver://localhost:1433;database=zjl;user=SA;password=*;trustServerCertificate=true;socket_timeout=60;isolationLevel=READ UNCOMMITTED".to_string(),
            }
        ];
        let cases = vec![
            Case {
                name: "new table".to_string(),
                schema: NEW_TABLE.to_string(),
            },
            Case {
                name: "update table comment".to_string(),
                schema: UPDATE_TABLE_COMMENT.to_string(),
            },
            Case {
                name: "update table comment and col".to_string(),
                schema: UPDATE_TABLE_COMMENT_AND_COL.to_string(),
            },
            Case {
                name: "update comment and add comment".to_string(),
                schema: UPDATE_COMMENT.to_string(),
            },
            Case {
                name: "update col type and add comment".to_string(),
                schema: UPDATE_COL.to_string(),
            },
            Case {
                name: "del col and add clo".to_string(),
                schema: DEL_COL.to_string(),
            },
        ];

        for v in ds {
            for c in &cases {
                let source = &v.to_string();
                let push_schema = source.to_string() + &*c.schema;
                println!("--------test: {}----------- \npush:\n{} ", c.name, push_schema);
                let res = test_push(&push_schema).await;
                println!("\nout_put:\n{:?}\n", res);
                // assert_eq!(res.executed_steps, c.steps);

                let intro_res = test_introspect(source).await;
                println!(
                    "introspect_res:\n{}\nwarnings:\n{:?}",
                    intro_res.datamodel, intro_res.warnings
                );
                let a = psl::parse_schema(intro_res.datamodel.as_str()).unwrap();
                let b = psl::parse_schema(push_schema.as_str()).unwrap();
                assert_eq!(format!("{:?}", a), format!("{:?}", b))
            }
        }
    }

    #[tokio::test]
    async fn test_in() {
        let intro_res = test_introspect(
            &r##"datasource db {
             provider = "mysql"
  url      = "mysql://root:*@127.0.0.1:3306/ttt"
               }"##
            .to_string(),
        )
        .await;
        println!(
            "introspect_res:\n{}\nwarnings:\n{:?}",
            intro_res.datamodel, intro_res.warnings
        );
    }

    #[tokio::test]
    async fn test_s() {
        tracing_subscriber::fmt().with_max_level(tracing::Level::DEBUG).init();
        let source = r##"
        datasource db {
  provider = "postgres"
  url      = "postgres:/*@localhost:5433/zjl"
}
model Account {
    membershipEndTime DateTime?
    createdAt DateTime @default(now())
    updatedAt DateTime
    deletedAt DateTime?
    membershipId String? @db.Uuid
    leftDuration Decimal @default(0) @db.Decimal(13, 3)
    typeId String
    id String @id @default(dbgenerated("gen_random_uuid()")) @db.Uuid
    type String @default("User")
    
    @@unique([type, typeId])
}
        "##;
        let res = test_push(&source).await;
        println!("\nout_put:\n{:?}\n", res);
        //
        // let intro_res = test_introspect(
        //     &r##"datasource db {
        //       provider = "mongodb"
        //   url      = "mongodb://root:*@localhost:27017/zjl?ssl=false&connectTimeoutMS=5000&maxPoolSize=50&authSource=admin"
        //
        //        }"##
        //     .to_string(),
        // )
        // .await;
        // println!(
        //     "introspect_res:\n{}\nwarnings:\n{:?}",
        //     intro_res.datamodel, intro_res.warnings
        // );
    }
    async fn test_introspect(schema: &String) -> IntrospectResult {
        let api = match schema_api(Some(schema.clone()), None) {
            Ok(api) => api,
            Err(e) => {
                panic!("schema_api meet err {}", e)
            }
        };

        let params = IntrospectParams {
            schema: schema.to_string(),
            force: false,
            composite_type_depth: 0,
            schemas: None,
        };

        let introspected = match api.introspect(params).await {
            Ok(introspected) => introspected,
            Err(e) => {
                panic!("schema_api meet err {:?}", e)
            }
        };
        introspected
    }

    async fn test_push(schema: &str) -> SchemaPushOutput {
        let api = match schema_api(Some(schema.to_string()), None) {
            Ok(api) => api,
            Err(e) => {
                panic!("schema_api meet err {}", e)
            }
        };

        let params = SchemaPushInput {
            schema: schema.to_string(),
            force: true,
        };
        let res = match api.schema_push(params).await.map_err(|err| println!("{}", err)) {
            Ok(introspected) => introspected,
            Err(e) => {
                panic!("schema_api meet err {:?}", e)
            }
        };

        res
    }

    #[tokio::test]
    async fn test_migration() {
        let s = String::from(
            r##"
datasource db {
  provider = "mysql"
  url      = ""
}

/// table comment
model User {
  /// id comment
  id    Int     @id @default(autoincrement())
  /// email comment
  email String  @unique
  /// posts comment
  posts Post[]
  /// author id
  authorID  Int
}

model Post {
    /// id comment
  id        Int     @id @default(autoincrement())
  /// id comment
  title     String
  /// id comment
  content   String?
  /// id comment
  published Boolean @default(false)
  /// id comment
  author    User    @relation(fields: [authorID], references: [id])
  authorID  Int
}
        "##,
        );
        tracing_subscriber::fmt().with_max_level(tracing::Level::DEBUG).init();
        let schema = String::from(s);
        let api = match schema_api(Some(schema.clone()), None) {
            Ok(api) => api,
            Err(e) => {
                panic!("schema_api meet err {}", e)
            }
        };

        let params = CreateMigrationInput {
            draft: false,
            migration_name: "test".to_string(),
            migrations_directory_path: "/Users/bytedance/RustroverProjects/prisma-engines/schema-engine/core/src"
                .to_string(),
            prisma_schema: schema,
        };

        let res = match api.create_migration(params).await.map_err(|err| panic!("{err:?}")) {
            Ok(introspected) => introspected,
            Err(e) => {
                panic!("schema_api meet err {:?}", e)
            }
        };

        println!("{:?}", res);
    }
}
