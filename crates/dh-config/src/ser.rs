//! Serializes any `serde::Serialize` type into a [`Value`] tree. This is how
//! the defaults layer turns a plain Rust struct into a mergeable layer.

use serde::ser::{self, Serialize};

use crate::error::ConfigError;
use crate::value::{Table, Value};

/// Converts any serializable value into a [`Value`] tree.
pub fn to_value<T: Serialize + ?Sized>(value: &T) -> Result<Value, ConfigError> {
    value.serialize(ValueSerializer)
}

struct ValueSerializer;

impl ser::Serializer for ValueSerializer {
    type Ok = Value;
    type Error = ConfigError;
    type SerializeSeq = SerializeArray;
    type SerializeTuple = SerializeArray;
    type SerializeTupleStruct = SerializeArray;
    type SerializeTupleVariant = SerializeTupleVariant;
    type SerializeMap = SerializeTable;
    type SerializeStruct = SerializeTable;
    type SerializeStructVariant = SerializeStructVariant;

    fn serialize_bool(self, v: bool) -> Result<Value, ConfigError> {
        Ok(Value::Bool(v))
    }

    fn serialize_i8(self, v: i8) -> Result<Value, ConfigError> {
        Ok(Value::Integer(v.into()))
    }

    fn serialize_i16(self, v: i16) -> Result<Value, ConfigError> {
        Ok(Value::Integer(v.into()))
    }

    fn serialize_i32(self, v: i32) -> Result<Value, ConfigError> {
        Ok(Value::Integer(v.into()))
    }

    fn serialize_i64(self, v: i64) -> Result<Value, ConfigError> {
        Ok(Value::Integer(v))
    }

    fn serialize_u8(self, v: u8) -> Result<Value, ConfigError> {
        Ok(Value::Integer(v.into()))
    }

    fn serialize_u16(self, v: u16) -> Result<Value, ConfigError> {
        Ok(Value::Integer(v.into()))
    }

    fn serialize_u32(self, v: u32) -> Result<Value, ConfigError> {
        Ok(Value::Integer(v.into()))
    }

    fn serialize_u64(self, v: u64) -> Result<Value, ConfigError> {
        i64::try_from(v)
            .map(Value::Integer)
            .map_err(|_| ser::Error::custom(format!("integer {v} does not fit in an i64")))
    }

    fn serialize_f32(self, v: f32) -> Result<Value, ConfigError> {
        Ok(Value::Float(v.into()))
    }

    fn serialize_f64(self, v: f64) -> Result<Value, ConfigError> {
        Ok(Value::Float(v))
    }

    fn serialize_char(self, v: char) -> Result<Value, ConfigError> {
        Ok(Value::String(v.to_string()))
    }

    fn serialize_str(self, v: &str) -> Result<Value, ConfigError> {
        Ok(Value::String(v.to_string()))
    }

    fn serialize_bytes(self, v: &[u8]) -> Result<Value, ConfigError> {
        Ok(Value::Array(
            v.iter().map(|b| Value::Integer((*b).into())).collect(),
        ))
    }

    fn serialize_none(self) -> Result<Value, ConfigError> {
        Ok(Value::Null)
    }

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<Value, ConfigError> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Value, ConfigError> {
        Ok(Value::Null)
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Value, ConfigError> {
        Ok(Value::Null)
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Value, ConfigError> {
        Ok(Value::String(variant.to_string()))
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Value, ConfigError> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Value, ConfigError> {
        let mut table = Table::new();
        table.insert(variant.to_string(), value.serialize(ValueSerializer)?);
        Ok(Value::Table(table))
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq, ConfigError> {
        Ok(SerializeArray {
            items: Vec::with_capacity(len.unwrap_or(0)),
        })
    }

    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, ConfigError> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleStruct, ConfigError> {
        self.serialize_seq(Some(len))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleVariant, ConfigError> {
        Ok(SerializeTupleVariant {
            variant,
            items: Vec::with_capacity(len),
        })
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, ConfigError> {
        Ok(SerializeTable {
            table: Table::new(),
            pending_key: None,
        })
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, ConfigError> {
        self.serialize_map(None)
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, ConfigError> {
        Ok(SerializeStructVariant {
            variant,
            table: Table::new(),
        })
    }
}

struct SerializeArray {
    items: Vec<Value>,
}

impl ser::SerializeSeq for SerializeArray {
    type Ok = Value;
    type Error = ConfigError;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), ConfigError> {
        self.items.push(value.serialize(ValueSerializer)?);
        Ok(())
    }

    fn end(self) -> Result<Value, ConfigError> {
        Ok(Value::Array(self.items))
    }
}

impl ser::SerializeTuple for SerializeArray {
    type Ok = Value;
    type Error = ConfigError;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), ConfigError> {
        ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Value, ConfigError> {
        ser::SerializeSeq::end(self)
    }
}

impl ser::SerializeTupleStruct for SerializeArray {
    type Ok = Value;
    type Error = ConfigError;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), ConfigError> {
        ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Value, ConfigError> {
        ser::SerializeSeq::end(self)
    }
}

struct SerializeTupleVariant {
    variant: &'static str,
    items: Vec<Value>,
}

impl ser::SerializeTupleVariant for SerializeTupleVariant {
    type Ok = Value;
    type Error = ConfigError;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), ConfigError> {
        self.items.push(value.serialize(ValueSerializer)?);
        Ok(())
    }

    fn end(self) -> Result<Value, ConfigError> {
        let mut table = Table::new();
        table.insert(self.variant.to_string(), Value::Array(self.items));
        Ok(Value::Table(table))
    }
}

struct SerializeTable {
    table: Table,
    pending_key: Option<String>,
}

impl ser::SerializeMap for SerializeTable {
    type Ok = Value;
    type Error = ConfigError;

    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), ConfigError> {
        self.pending_key = Some(key.serialize(KeySerializer)?);
        Ok(())
    }

    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), ConfigError> {
        let key = self
            .pending_key
            .take()
            .expect("serialize_value called before serialize_key");
        self.table.insert(key, value.serialize(ValueSerializer)?);
        Ok(())
    }

    fn end(self) -> Result<Value, ConfigError> {
        Ok(Value::Table(self.table))
    }
}

impl ser::SerializeStruct for SerializeTable {
    type Ok = Value;
    type Error = ConfigError;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), ConfigError> {
        self.table
            .insert(key.to_string(), value.serialize(ValueSerializer)?);
        Ok(())
    }

    fn end(self) -> Result<Value, ConfigError> {
        Ok(Value::Table(self.table))
    }
}

struct SerializeStructVariant {
    variant: &'static str,
    table: Table,
}

impl ser::SerializeStructVariant for SerializeStructVariant {
    type Ok = Value;
    type Error = ConfigError;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), ConfigError> {
        self.table
            .insert(key.to_string(), value.serialize(ValueSerializer)?);
        Ok(())
    }

    fn end(self) -> Result<Value, ConfigError> {
        let mut outer = Table::new();
        outer.insert(self.variant.to_string(), Value::Table(self.table));
        Ok(Value::Table(outer))
    }
}

/// Map keys must serialize to strings; integers are stringified for
/// convenience since `HashMap<u32, _>` configs are common.
struct KeySerializer;

macro_rules! key_to_string {
    ($($method:ident: $ty:ty),* $(,)?) => {
        $(fn $method(self, v: $ty) -> Result<String, ConfigError> {
            Ok(v.to_string())
        })*
    };
}

macro_rules! key_unsupported {
    ($($method:ident$(($($arg:ty),*))?),* $(,)?) => {
        $(fn $method(self $($(, _: $arg)*)?) -> Result<Self::Ok, Self::Error> {
            Err(ser::Error::custom("map keys must be strings"))
        })*
    };
}

impl ser::Serializer for KeySerializer {
    type Ok = String;
    type Error = ConfigError;
    type SerializeSeq = ser::Impossible<String, ConfigError>;
    type SerializeTuple = ser::Impossible<String, ConfigError>;
    type SerializeTupleStruct = ser::Impossible<String, ConfigError>;
    type SerializeTupleVariant = ser::Impossible<String, ConfigError>;
    type SerializeMap = ser::Impossible<String, ConfigError>;
    type SerializeStruct = ser::Impossible<String, ConfigError>;
    type SerializeStructVariant = ser::Impossible<String, ConfigError>;

    key_to_string! {
        serialize_bool: bool,
        serialize_i8: i8,
        serialize_i16: i16,
        serialize_i32: i32,
        serialize_i64: i64,
        serialize_u8: u8,
        serialize_u16: u16,
        serialize_u32: u32,
        serialize_u64: u64,
        serialize_char: char,
    }

    fn serialize_str(self, v: &str) -> Result<String, ConfigError> {
        Ok(v.to_string())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<String, ConfigError> {
        Ok(variant.to_string())
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<String, ConfigError> {
        value.serialize(self)
    }

    key_unsupported! {
        serialize_f32(f32),
        serialize_f64(f64),
        serialize_bytes(&[u8]),
        serialize_none,
        serialize_unit,
        serialize_unit_struct(&'static str),
    }

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<String, ConfigError> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<String, ConfigError> {
        Err(ser::Error::custom("map keys must be strings"))
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, ConfigError> {
        Err(ser::Error::custom("map keys must be strings"))
    }

    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, ConfigError> {
        Err(ser::Error::custom("map keys must be strings"))
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, ConfigError> {
        Err(ser::Error::custom("map keys must be strings"))
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, ConfigError> {
        Err(ser::Error::custom("map keys must be strings"))
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, ConfigError> {
        Err(ser::Error::custom("map keys must be strings"))
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, ConfigError> {
        Err(ser::Error::custom("map keys must be strings"))
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, ConfigError> {
        Err(ser::Error::custom("map keys must be strings"))
    }
}
