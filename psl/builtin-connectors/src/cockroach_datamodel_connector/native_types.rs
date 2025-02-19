use crate::geometry::GeometryParams;

crate::native_type_definition! {
    CockroachType;
    Bit(Option<u32>) -> String,
    Bool -> Boolean,
    Bytes -> Bytes,
    Char(Option<u32>) -> String,
    Date -> DateTime,
    Decimal(Option<(u32, u32)>) -> Decimal,
    Float4 -> Float,
    Float8 -> Float,
    Inet -> String,
    Int2 -> Int,
    Int4 -> Int,
    Int8 -> BigInt,
    JsonB -> Json,
    Oid -> Int,
    CatalogSingleChar -> String,
    String(Option<u32>) -> String,
    Time(Option<u32>) -> DateTime,
    Timestamp(Option<u32>) -> DateTime,
    Timestamptz(Option<u32>) -> DateTime,
    Timetz(Option<u32>) -> DateTime,
    Uuid -> String,
    VarBit(Option<u32>) -> String,
    Geometry(Option<GeometryParams>) -> Geometry | GeoJson,
    Geography(Option<GeometryParams>) -> Geometry | GeoJson,
}
