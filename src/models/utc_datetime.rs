use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Decode, Encode, Sqlite, Type};

#[derive(Debug, PartialEq, Serialize, Deserialize, Clone)]
pub struct UtcDateTime(pub DateTime<Utc>);

impl Type<Sqlite> for UtcDateTime {
    fn type_info() -> <Sqlite as sqlx::Database>::TypeInfo {
        // SQLite stores datetimes as TEXT, so we declare TEXT as our type
        <String as Type<Sqlite>>::type_info()
    }

    fn compatible(ty: &<Sqlite as sqlx::Database>::TypeInfo) -> bool {
        // Accept both TEXT and DATETIME types for flexibility
        <String as Type<Sqlite>>::compatible(ty) || <NaiveDateTime as Type<Sqlite>>::compatible(ty)
    }
}

impl<'r> Decode<'r, Sqlite> for UtcDateTime {
    fn decode(
        value: <Sqlite as sqlx::database::HasValueRef<'r>>::ValueRef,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        // Decode as string and parse - handles TEXT storage in SQLite
        let s = <String as Decode<Sqlite>>::decode(value)?;
        let naive = NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S")
            .or_else(|_| NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%S"))
            .or_else(|_| NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%S%.f"))
            .or_else(|_| NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S%.f"))?;
        Ok(UtcDateTime(naive.and_utc()))
    }
}

impl<'q> Encode<'q, Sqlite> for UtcDateTime {
    fn encode_by_ref(
        &self,
        buf: &mut <Sqlite as sqlx::database::HasArguments<'q>>::ArgumentBuffer,
    ) -> sqlx::encode::IsNull {
        // Encode as TEXT in ISO format for SQLite compatibility
        let s = self.0.naive_utc().format("%Y-%m-%d %H:%M:%S").to_string();
        <String as Encode<Sqlite>>::encode_by_ref(&s, buf)
    }
}

impl From<NaiveDateTime> for UtcDateTime {
    fn from(naive: NaiveDateTime) -> Self {
        UtcDateTime(naive.and_utc())
    }
}

impl From<UtcDateTime> for NaiveDateTime {
    fn from(utc_datetime: UtcDateTime) -> Self {
        utc_datetime.0.naive_utc()
    }
}

impl From<DateTime<Utc>> for UtcDateTime {
    fn from(dt: DateTime<Utc>) -> Self {
        UtcDateTime(dt)
    }
}

impl From<UtcDateTime> for DateTime<Utc> {
    fn from(utc_datetime: UtcDateTime) -> Self {
        utc_datetime.0
    }
}

pub fn now() -> UtcDateTime {
    UtcDateTime(Utc::now())
}
