use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Decode, Encode, Sqlite, Type};

#[derive(Debug, PartialEq, Serialize, Deserialize, Clone)]
pub struct UtcDateTime(pub DateTime<Utc>);

impl Type<Sqlite> for UtcDateTime {
    fn type_info() -> <Sqlite as sqlx::Database>::TypeInfo {
        <NaiveDateTime as Type<Sqlite>>::type_info()
    }
}

impl<'r> Decode<'r, Sqlite> for UtcDateTime {
    fn decode(
        value: <Sqlite as sqlx::database::HasValueRef<'r>>::ValueRef,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        let naive = <NaiveDateTime as Decode<Sqlite>>::decode(value)?;
        Ok(UtcDateTime(naive.and_utc()))
    }
}

impl<'q> Encode<'q, Sqlite> for UtcDateTime {
    fn encode_by_ref(
        &self,
        buf: &mut <Sqlite as sqlx::database::HasArguments<'q>>::ArgumentBuffer,
    ) -> sqlx::encode::IsNull {
        let naive = self.0.naive_utc();
        <NaiveDateTime as Encode<Sqlite>>::encode_by_ref(&naive, buf)
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
