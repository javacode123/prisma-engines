use super::render_default;
use crate::{
    flavour::MssqlFlavour,
    migration_pair::MigrationPair,
    sql_migration::AlterColumn,
    sql_migration::TableChange,
    sql_renderer::{
        common::{IteratorJoin, Quoted},
        SqlRenderer,
    },
    sql_schema_differ::ColumnChanges,
};
use sql_schema_describer::{
    mssql::MssqlSchemaExt,
    walkers::{TableColumnWalker, TableWalker},
    DefaultValue, TableColumnId,
};
use std::borrow::Cow;
use std::collections::BTreeSet;

/// Creates a set of `ALTER TABLE` statements in a correct execution order.
pub(crate) fn create_statements(
    renderer: &MssqlFlavour,
    tables: MigrationPair<TableWalker<'_>>,
    changes: &[TableChange],
) -> Vec<String> {
    let constructor = AlterTableConstructor {
        renderer,
        tables,
        changes,
        drop_constraints: BTreeSet::new(),
        add_constraints: BTreeSet::new(),
        rename_primary_key: false,
        add_columns: Vec::new(),
        drop_columns: Vec::new(),
        column_mods: Vec::new(),
        comments: Vec::new(),
    };

    constructor.into_statements()
}

struct AlterTableConstructor<'a> {
    renderer: &'a MssqlFlavour,
    tables: MigrationPair<TableWalker<'a>>,
    changes: &'a [TableChange],
    drop_constraints: BTreeSet<String>,
    add_constraints: BTreeSet<String>,
    rename_primary_key: bool,
    add_columns: Vec<String>,
    drop_columns: Vec<String>,
    column_mods: Vec<String>,
    comments: Vec<String>,
}

impl<'a> AlterTableConstructor<'a> {
    fn into_statements(mut self) -> Vec<String> {
        for change in self.changes {
            match change {
                TableChange::DropPrimaryKey => {
                    self.drop_primary_key();
                }
                TableChange::RenamePrimaryKey => {
                    self.rename_primary_key = true;
                }
                TableChange::AddPrimaryKey => {
                    self.add_primary_key();
                }
                TableChange::AddColumn {
                    column_id,
                    has_virtual_default: _,
                } => {
                    self.add_column(*column_id);
                }
                TableChange::DropColumn { column_id } => {
                    self.drop_column(*column_id);
                }
                TableChange::DropAndRecreateColumn { column_id, .. } => {
                    self.drop_and_recreate_column(*column_id);
                }
                TableChange::AlterColumn(AlterColumn {
                    column_id,
                    changes,
                    type_change: _,
                }) => {
                    self.alter_column(*column_id, changes);
                }
                TableChange::AlterComment { .. } => {
                    // 更新表注释
                    self.alter_table_comment();
                }
            };
        }

        // Order matters
        let mut statements = Vec::new();

        if !self.drop_constraints.is_empty() {
            statements.push(format!(
                "ALTER TABLE {} DROP CONSTRAINT {}",
                self.renderer.table_name(self.tables.previous),
                self.drop_constraints.iter().join(",\n"),
            ));
        }

        if self.rename_primary_key {
            let with_schema = format!(
                "{}.{}",
                self.tables
                    .previous
                    .namespace()
                    .unwrap_or_else(|| self.renderer.schema_name()),
                self.tables.previous.primary_key().unwrap().name()
            );

            statements.push(format!(
                "EXEC SP_RENAME N{}, N{}",
                Quoted::mssql_string(with_schema),
                Quoted::mssql_string(self.tables.next.primary_key().unwrap().name()),
            ));
        }

        if !self.column_mods.is_empty() {
            statements.extend(self.column_mods)
        }

        if !self.drop_columns.is_empty() {
            statements.push(format!(
                "ALTER TABLE {} DROP COLUMN {}",
                self.renderer.table_name(self.tables.previous),
                self.drop_columns.join(",\n"),
            ));
        }

        if !self.add_constraints.is_empty() {
            statements.push(format!(
                "ALTER TABLE {} ADD {}",
                self.renderer.table_name(self.tables.previous),
                self.add_constraints.iter().join(", ")
            ));
        }

        if !self.add_columns.is_empty() {
            statements.push(format!(
                "ALTER TABLE {} ADD {}",
                self.renderer.table_name(self.tables.previous),
                self.add_columns.join(",\n"),
            ));
        }

        if !self.comments.is_empty() {
            statements.extend(self.comments)
        }

        statements
    }

    fn drop_primary_key(&mut self) {
        let constraint_name = self
            .tables
            .previous
            .primary_key()
            .map(|pk| pk.name())
            .expect("Missing constraint name in DropPrimaryKey on MSSQL");

        self.drop_constraints
            .insert(format!("{}", self.renderer.quote(constraint_name)));
    }

    fn add_primary_key(&mut self) {
        let mssql_schema_ext: &MssqlSchemaExt = self.tables.next.schema.downcast_connector_data();
        let next_pk = self.tables.next.primary_key().unwrap();

        let columns = self.tables.next.primary_key_columns().unwrap();
        let mut quoted_columns = Vec::new();

        for column in columns {
            let mut rendered = Quoted::mssql_ident(column.as_column().name()).to_string();

            if let Some(sort_order) = column.sort_order() {
                rendered.push(' ');
                rendered.push_str(sort_order.as_ref());
            }

            quoted_columns.push(rendered);
        }

        let clustering = if mssql_schema_ext.index_is_clustered(next_pk.id) {
            " CLUSTERED"
        } else {
            " NONCLUSTERED"
        };

        self.add_constraints.insert(format!(
            "CONSTRAINT {} PRIMARY KEY{} ({})",
            next_pk.name(),
            clustering,
            quoted_columns.join(","),
        ));
    }

    fn add_column(&mut self, column_id: TableColumnId) {
        let column = self.tables.next.schema.walk(column_id);
        self.add_columns.push(self.renderer.render_column(column));

        if let Some(comment) = column.description() {
            self.comments.push(format!(
                "BEGIN TRY
                    EXEC sp_addextendedproperty 
                        @name = N'MS_Description', 
                        @value = '{}',
                        @level0type = N'SCHEMA', 
                        @level0name = dbo, 
                        @level1type = N'TABLE', 
                        @level1name = '{}', 
                        @level2type = N'COLUMN', 
                        @level2name = '{}';
                END TRY
                BEGIN CATCH
                    EXEC sp_updateextendedproperty 
                        @name = N'MS_Description', 
                        @value = '{}',
                        @level0type = N'SCHEMA', 
                        @level0name = dbo, 
                        @level1type = N'TABLE', 
                        @level1name = '{}', 
                        @level2type = N'COLUMN', 
                        @level2name = '{}';
                END CATCH
                ",
                comment,
                column.table().name(),
                column.name(),
                comment,
                column.table().name(),
                column.name(),
            ));
        }
    }

    fn drop_column(&mut self, column_id: TableColumnId) {
        let column = self.tables.previous.walk(column_id);
        let name = self.renderer.quote(column.name());

        self.drop_columns.push(format!("{name}"));
        // 无脑删除 comment, 忽略异常
        self.comments.push(format!(
            "BEGIN TRY
                EXEC sp_dropextendedproperty
                @name = N'MS_Description',
                @level0type = N'SCHEMA',
                @level0name = 'dbo',
                @level2type = N'COLUMN',
                @level2name = '{}',
                @level1type = N'TABLE', 
                @level1name = '{}';
            END TRY
            BEGIN CATCH
                -- 错误处理代码，如果你想要完全忽略错误，你可以让这里什么都不做
            END CATCH",
            column.name(),
            column.table().name()
        ));
    }

    fn drop_and_recreate_column(&mut self, columns: MigrationPair<TableColumnId>) {
        let columns = self.tables.walk(columns);

        self.drop_columns
            .push(format!("{}", self.renderer.quote(columns.previous.name())));

        self.add_columns.push(self.renderer.render_column(columns.next));
    }

    fn alter_column(&mut self, columns: MigrationPair<TableColumnId>, changes: &ColumnChanges) {
        let columns = self.tables.walk(columns);
        let expanded = expand_alter_column(&columns, changes);

        for alter in expanded.into_iter() {
            match alter {
                MsSqlAlterColumn::DropDefault { constraint_name } => {
                    let escaped = format!("{}", self.renderer.quote(&constraint_name));
                    self.drop_constraints.insert(escaped);
                }
                MsSqlAlterColumn::SetDefault(default) => {
                    let constraint_name = default.constraint_name().map(Cow::from).unwrap_or_else(|| {
                        let old_default = format!("DF__{}__{}", self.tables.next.name(), columns.next.name());
                        Cow::from(old_default)
                    });

                    let default = render_default(&default);

                    self.add_constraints.insert(format!(
                        "CONSTRAINT [{constraint}] DEFAULT {default} FOR [{column}]",
                        constraint = constraint_name,
                        column = columns.next.name(),
                        default = default,
                    ));
                }
                MsSqlAlterColumn::Modify => {
                    let nullability = if columns.next.arity().is_required() {
                        "NOT NULL"
                    } else {
                        "NULL"
                    };

                    self.column_mods.push(format!(
                        "ALTER TABLE {table} ALTER COLUMN {column_name} {column_type} {nullability}",
                        table = self.renderer.table_name(self.tables.previous),
                        column_name = self.renderer.quote(columns.next.name()),
                        column_type = super::render_column_type(columns.next),
                        nullability = nullability,
                    ));
                }
                MsSqlAlterColumn::AlterComment => {
                    match columns.previous.description() {
                        Some(_) => {
                            // 之前就有 comment
                            match columns.next.description() {
                                Some(comment) => {
                                    // 更新 comment
                                    self.comments.push(format!(
                                        r##"
                                        EXEC sp_updateextendedproperty 
                                            @name = N'MS_Description', 
                                            @value = '{}', 
                                            @level0type = N'SCHEMA', 
                                            @level0name = 'dbo', 
                                            @level1type = N'TABLE', 
                                            @level1name = '{}', 
                                            @level2type = N'COLUMN', 
                                            @level2name = '{}';
                                    "##,
                                        comment,
                                        self.tables.next.name(),
                                        columns.next.name()
                                    ))
                                }
                                None => {
                                    // 删除注释
                                    self.comments.push(format!(
                                        r##"
                                        EXEC sp_dropextendedproperty 
                                            @name = N'MS_Description', 
                                            @level0type = N'SCHEMA', 
                                            @level0name = 'dbo', 
                                            @level1type = N'TABLE', 
                                            @level1name = '{}', 
                                            @level2type = N'COLUMN', 
                                            @level2name = '{}';
                                    "##,
                                        self.tables.next.name(),
                                        columns.next.name()
                                    ))
                                }
                            }
                        }
                        None => {
                            // 无 comment
                            match columns.next.description() {
                                Some(comment) => {
                                    // 新增 comment
                                    self.comments.push(format!(
                                        r##"
                                        EXEC sp_addextendedproperty 
                                            @name = N'MS_Description', 
                                            @value = '{}', 
                                            @level0type = N'SCHEMA', 
                                            @level0name = 'dbo', 
                                            @level1type = N'TABLE', 
                                            @level1name = '{}', 
                                            @level2type = N'COLUMN', 
                                            @level2name = '{}';
                                    "##,
                                        comment,
                                        self.tables.next.name(),
                                        columns.next.name()
                                    ))
                                }
                                None => {
                                    // 不存在 case
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn alter_table_comment(&mut self) {
        match self.tables.previous.description() {
            Some(_) => {
                //之前存在 comment
                match self.tables.next.description() {
                    Some(comment) => {
                        // 更新
                        self.comments.push(format!(
                            r##"EXEC sp_updateextendedproperty 
                                        @name = N'MS_Description', 
                                        @value = '{}', 
                                        @level0type = N'SCHEMA', 
                                        @level0name = 'dbo', 
                                        @level1type = N'TABLE', 
                                        @level1name = '{}';"##,
                            comment.trim(),
                            self.tables.next.name(),
                        ))
                    }
                    None => {
                        // 删除
                        self.comments.push(format!(
                            r##"EXEC sp_dropextendedproperty 
                                    @name = N'MS_Description', 
                                    @level0type = N'SCHEMA', 
                                    @level0name = 'dbo', 
                                    @level1type = N'TABLE', 
                                    @level1name = '{}';"##,
                            self.tables.next.name(),
                        ))
                    }
                }
            }
            None => {
                // 之前不存在 comment
                match self.tables.next.description() {
                    Some(comment) => {
                        // 添加
                        self.comments.push(format!(
                            r##"EXEC sp_addextendedproperty 
                                        @name = N'MS_Description', 
                                        @value = '{}', 
                                        @level0type = N'SCHEMA', 
                                        @level0name = 'dbo', 
                                        @level1type = N'TABLE', 
                                        @level1name = '{}';"##,
                            comment.trim(),
                            self.tables.next.name(),
                        ))
                    }
                    None => {
                        // 删除
                    }
                }
            }
        }
    }
}

#[derive(Debug)]
enum MsSqlAlterColumn {
    DropDefault { constraint_name: String },
    SetDefault(DefaultValue),
    Modify,
    AlterComment,
}

fn expand_alter_column(
    columns: &MigrationPair<TableColumnWalker<'_>>,
    column_changes: &ColumnChanges,
) -> Vec<MsSqlAlterColumn> {
    let mut changes = Vec::new();

    if column_changes.only_comment_changed() {
        // 此行只更新了 comment
        changes.push(MsSqlAlterColumn::AlterComment);
        return changes;
    }

    // Default value changes require us to re-create the constraint, which we
    // must do before modifying the column.
    if column_changes.default_changed() {
        if let Some(default) = columns.previous.default() {
            let constraint_name = default.constraint_name();

            changes.push(MsSqlAlterColumn::DropDefault {
                constraint_name: constraint_name.unwrap().into(),
            });
        }

        if !column_changes.only_default_changed() {
            changes.push(MsSqlAlterColumn::Modify);
        }

        if let Some(next_default) = columns.next.default() {
            changes.push(MsSqlAlterColumn::SetDefault(next_default.inner().clone()));
        }
    } else {
        changes.push(MsSqlAlterColumn::Modify);
    }

    if column_changes.comment_changed() {
        // 伴随更新了 comment
        changes.push(MsSqlAlterColumn::AlterComment);
    }

    changes
}
