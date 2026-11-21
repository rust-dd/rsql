use tokio_postgres::types::{Format, IsNull, ToSql, Type, to_sql_checked};

#[derive(Debug)]
pub(super) struct TextParam<'a>(pub Option<&'a str>);

impl ToSql for TextParam<'_> {
    fn to_sql(
        &self,
        _: &Type,
        out: &mut bytes::BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        match self.0 {
            Some(value) => {
                out.extend_from_slice(value.as_bytes());
                Ok(IsNull::No)
            }
            None => Ok(IsNull::Yes),
        }
    }

    fn accepts(_: &Type) -> bool {
        true
    }

    fn encode_format(&self, _: &Type) -> Format {
        Format::Text
    }

    to_sql_checked!();
}

pub(super) fn captured_type(oid: u32) -> Type {
    Type::new(
        String::new(),
        oid,
        tokio_postgres::types::Kind::Simple,
        String::new(),
    )
}
