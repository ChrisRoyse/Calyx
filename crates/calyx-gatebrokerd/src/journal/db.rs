use std::path::Path;

use turso::{Connection, Database, Value, params_from_iter};

use super::JournalError;

pub(super) struct JournalDb {
    runtime: tokio::runtime::Runtime,
    _database: Database,
    connection: Connection,
}

pub(super) struct JournalTransaction<'a> {
    database: &'a JournalDb,
    finished: bool,
}

#[derive(Clone, Debug)]
pub(super) struct DbRow {
    values: Vec<Value>,
}

pub(super) trait QueryExecutor {
    fn execute(&self, sql: &str, params: Vec<Value>) -> Result<u64, turso::Error>;
    fn query(&self, sql: &str, params: Vec<Value>) -> Result<Vec<DbRow>, turso::Error>;

    fn query_optional(&self, sql: &str, params: Vec<Value>) -> Result<Option<DbRow>, turso::Error> {
        let mut rows = self.query(sql, params)?;
        if rows.len() > 1 {
            return Err(turso::Error::Misuse(format!(
                "query expected at most one row, returned {}",
                rows.len()
            )));
        }
        Ok(rows.pop())
    }

    fn query_one(&self, sql: &str, params: Vec<Value>) -> Result<DbRow, turso::Error> {
        self.query_optional(sql, params)?
            .ok_or(turso::Error::QueryReturnedNoRows)
    }
}

impl JournalDb {
    pub(super) fn open(path: &Path) -> Result<Self, JournalError> {
        let path = path.to_str().ok_or_else(|| {
            JournalError::InvalidPath(format!("{} is not valid UTF-8", path.display()))
        })?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|source| {
                JournalError::Durability(format!("create Turso runtime: {source}"))
            })?;
        let database = runtime
            .block_on(turso::Builder::new_local(path).build())
            .map_err(|source| JournalError::Sql {
                operation: "open journal",
                source,
            })?;
        let connection = database.connect().map_err(|source| JournalError::Sql {
            operation: "connect journal",
            source,
        })?;
        Ok(Self {
            runtime,
            _database: database,
            connection,
        })
    }

    pub(super) fn execute_batch(&self, sql: &str) -> Result<(), turso::Error> {
        self.runtime.block_on(self.connection.execute_batch(sql))
    }

    pub(super) fn transaction(&self) -> Result<JournalTransaction<'_>, turso::Error> {
        self.execute("BEGIN IMMEDIATE", Vec::new())?;
        Ok(JournalTransaction {
            database: self,
            finished: false,
        })
    }

    fn query_inner(&self, sql: &str, params: Vec<Value>) -> Result<Vec<DbRow>, turso::Error> {
        self.runtime.block_on(async {
            let mut rows = self.connection.query(sql, params_from_iter(params)).await?;
            let column_count = rows.column_count();
            let mut output = Vec::new();
            while let Some(row) = rows.next().await? {
                let mut values = Vec::with_capacity(column_count);
                for index in 0..column_count {
                    values.push(row.get_value(index)?);
                }
                output.push(DbRow { values });
            }
            Ok(output)
        })
    }
}

impl QueryExecutor for JournalDb {
    fn execute(&self, sql: &str, params: Vec<Value>) -> Result<u64, turso::Error> {
        self.runtime
            .block_on(self.connection.execute(sql, params_from_iter(params)))
    }

    fn query(&self, sql: &str, params: Vec<Value>) -> Result<Vec<DbRow>, turso::Error> {
        self.query_inner(sql, params)
    }
}

impl QueryExecutor for JournalTransaction<'_> {
    fn execute(&self, sql: &str, params: Vec<Value>) -> Result<u64, turso::Error> {
        self.database.execute(sql, params)
    }

    fn query(&self, sql: &str, params: Vec<Value>) -> Result<Vec<DbRow>, turso::Error> {
        self.database.query(sql, params)
    }
}

impl JournalTransaction<'_> {
    pub(super) fn commit(mut self) -> Result<(), turso::Error> {
        self.database.execute("COMMIT", Vec::new())?;
        self.finished = true;
        Ok(())
    }
}

impl Drop for JournalTransaction<'_> {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.database.execute("ROLLBACK", Vec::new());
        }
    }
}

impl DbRow {
    pub(super) fn get<T: FromDbValue>(&self, index: usize) -> Result<T, turso::Error> {
        let value = self.values.get(index).ok_or_else(|| {
            turso::Error::Misuse(format!(
                "column index {index} out of bounds for {} values",
                self.values.len()
            ))
        })?;
        T::from_db(value)
    }
}

pub(super) trait IntoDbValue {
    fn into_db_value(self) -> Result<Value, turso::Error>;
}

pub(super) trait FromDbValue: Sized {
    fn from_db(value: &Value) -> Result<Self, turso::Error>;
}

impl IntoDbValue for Value {
    fn into_db_value(self) -> Result<Value, turso::Error> {
        Ok(self)
    }
}

impl IntoDbValue for &str {
    fn into_db_value(self) -> Result<Value, turso::Error> {
        Ok(Value::Text(self.to_string()))
    }
}

impl IntoDbValue for String {
    fn into_db_value(self) -> Result<Value, turso::Error> {
        Ok(Value::Text(self))
    }
}

impl IntoDbValue for &String {
    fn into_db_value(self) -> Result<Value, turso::Error> {
        Ok(Value::Text(self.clone()))
    }
}

impl IntoDbValue for &[u8] {
    fn into_db_value(self) -> Result<Value, turso::Error> {
        Ok(Value::Blob(self.to_vec()))
    }
}

impl IntoDbValue for Vec<u8> {
    fn into_db_value(self) -> Result<Value, turso::Error> {
        Ok(Value::Blob(self))
    }
}

impl IntoDbValue for &Vec<u8> {
    fn into_db_value(self) -> Result<Value, turso::Error> {
        Ok(Value::Blob(self.clone()))
    }
}

macro_rules! integer_value {
    ($($type:ty),+ $(,)?) => {
        $(
            impl IntoDbValue for $type {
                fn into_db_value(self) -> Result<Value, turso::Error> {
                    i64::try_from(self)
                        .map(Value::Integer)
                        .map_err(|_| turso::Error::ConversionFailure(
                            format!("integer {} exceeds i64", self)
                        ))
                }
            }
        )+
    };
}

integer_value!(i64, i32, u64, u32, usize);

impl<T: IntoDbValue> IntoDbValue for Option<T> {
    fn into_db_value(self) -> Result<Value, turso::Error> {
        match self {
            Some(value) => value.into_db_value(),
            None => Ok(Value::Null),
        }
    }
}

impl FromDbValue for String {
    fn from_db(value: &Value) -> Result<Self, turso::Error> {
        match value {
            Value::Text(value) => Ok(value.clone()),
            _ => Err(conversion("text", value)),
        }
    }
}

impl FromDbValue for i64 {
    fn from_db(value: &Value) -> Result<Self, turso::Error> {
        match value {
            Value::Integer(value) => Ok(*value),
            _ => Err(conversion("integer", value)),
        }
    }
}

impl FromDbValue for u32 {
    fn from_db(value: &Value) -> Result<Self, turso::Error> {
        let value = i64::from_db(value)?;
        value
            .try_into()
            .map_err(|_| turso::Error::ConversionFailure(format!("{value} is outside u32")))
    }
}

impl FromDbValue for i32 {
    fn from_db(value: &Value) -> Result<Self, turso::Error> {
        let value = i64::from_db(value)?;
        value
            .try_into()
            .map_err(|_| turso::Error::ConversionFailure(format!("{value} is outside i32")))
    }
}

impl FromDbValue for Vec<u8> {
    fn from_db(value: &Value) -> Result<Self, turso::Error> {
        match value {
            Value::Blob(value) => Ok(value.clone()),
            _ => Err(conversion("blob", value)),
        }
    }
}

impl<T: FromDbValue> FromDbValue for Option<T> {
    fn from_db(value: &Value) -> Result<Self, turso::Error> {
        match value {
            Value::Null => Ok(None),
            value => T::from_db(value).map(Some),
        }
    }
}

fn conversion(expected: &str, actual: &Value) -> turso::Error {
    turso::Error::ConversionFailure(format!("expected {expected}, got {actual:?}"))
}

macro_rules! db_params {
    () => {
        Ok::<Vec<turso::Value>, turso::Error>(Vec::new())
    };
    ($($value:expr),+ $(,)?) => {{
        (|| -> Result<Vec<turso::Value>, turso::Error> {
            Ok(vec![$($crate::journal::db::IntoDbValue::into_db_value($value)?),+])
        })()
    }};
}

pub(super) use db_params;
