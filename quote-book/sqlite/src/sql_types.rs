// Copyright (c) 2023 MobileCoin Inc.

use diesel::{
    backend,
    deserialize::{self, FromSql},
    serialize::{self, Output, ToSql},
    sqlite::Sqlite,
    AsExpression, FromSqlRow,
};
use std::{fmt, ops::Deref};

/// SQLite type wrapper for storing a u64 in a binary column.
/// This helps us get around the limitation of SQLite not supporting unsigned 64
/// integers. By storing our u64s as its big endian bytes in a binary blob, we
/// can still perform sorting and comparisons on the column, since SQLite uses
/// memcmp() to compare binary blobs.
#[derive(AsExpression, FromSqlRow, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[diesel(sql_type = diesel::sql_types::Binary)]
pub struct VecU64(u64);

impl Deref for VecU64 {
    type Target = u64;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<u64> for VecU64 {
    fn from(src: u64) -> Self {
        Self(src)
    }
}

impl From<&u64> for VecU64 {
    fn from(src: &u64) -> Self {
        Self(*src)
    }
}

impl fmt::Display for VecU64 {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromSql<diesel::sql_types::Binary, Sqlite> for VecU64 {
    fn from_sql(bytes: backend::RawValue<'_, Sqlite>) -> deserialize::Result<Self> {
        let vec = <Vec<u8> as FromSql<diesel::sql_types::Binary, Sqlite>>::from_sql(bytes)?;
        if vec.len() != 8 {
            return Err("VecU64: Invalid array length".into());
        }

        let mut bytes = [0; 8];
        bytes.copy_from_slice(&vec);

        Ok(VecU64(u64::from_be_bytes(bytes)))
    }
}

impl ToSql<diesel::sql_types::Binary, Sqlite> for VecU64 {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Sqlite>) -> serialize::Result {
        let bytes = self.0.to_be_bytes().to_vec();
        out.set_value(bytes);
        Ok(serialize::IsNull::No)
    }
}
