//! Serde support for [`Value`]: parsing any self-describing format *into* a
//! `Value`, and deserializing a `Value` *out* into user-defined types.

use std::collections::btree_map;
use std::fmt;

use serde::de::{
    self, Deserialize, DeserializeSeed, Deserializer, EnumAccess, IntoDeserializer, MapAccess,
    SeqAccess, VariantAccess, Visitor,
};

use crate::error::ConfigError;
use crate::value::{Table, Value};

/// The field name the `toml` crate uses when serializing datetimes through
/// `deserialize_any`. We unwrap it back into a plain string so TOML datetimes
/// surface as strings rather than a one-entry magic table.
const TOML_DATETIME_KEY: &str = "$__toml_private_datetime";

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ValueVisitor;

        impl<'de> Visitor<'de> for ValueVisitor {
            type Value = Value;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("any configuration value")
            }

            fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
                Ok(Value::Bool(v))
            }

            fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
                Ok(Value::Integer(v))
            }

            fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
                Ok(match i64::try_from(v) {
                    Ok(v) => Value::Integer(v),
                    Err(_) => Value::Float(v as f64),
                })
            }

            fn visit_f64<E>(self, v: f64) -> Result<Value, E> {
                Ok(Value::Float(v))
            }

            fn visit_str<E>(self, v: &str) -> Result<Value, E> {
                Ok(Value::String(v.to_string()))
            }

            fn visit_string<E>(self, v: String) -> Result<Value, E> {
                Ok(Value::String(v))
            }

            fn visit_none<E>(self) -> Result<Value, E> {
                Ok(Value::Null)
            }

            fn visit_unit<E>(self) -> Result<Value, E> {
                Ok(Value::Null)
            }

            fn visit_some<D>(self, deserializer: D) -> Result<Value, D::Error>
            where
                D: Deserializer<'de>,
            {
                Value::deserialize(deserializer)
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element()? {
                    items.push(item);
                }
                Ok(Value::Array(items))
            }

            fn visit_map<A>(self, mut map: A) -> Result<Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut table = Table::new();
                while let Some((key, value)) = map.next_entry::<String, Value>()? {
                    table.insert(key, value);
                }
                if table.len() == 1 {
                    if let Some(Value::String(datetime)) = table.get(TOML_DATETIME_KEY) {
                        return Ok(Value::String(datetime.clone()));
                    }
                }
                Ok(Value::Table(table))
            }
        }

        deserializer.deserialize_any(ValueVisitor)
    }
}

/// A [`Deserializer`] over a borrowed [`Value`] tree.
///
/// Tracks the dot-separated path it has descended through so type errors can
/// name the offending key.
pub struct ValueDeserializer<'de> {
    value: &'de Value,
    path: String,
}

impl<'de> ValueDeserializer<'de> {
    /// Creates a deserializer rooted at `value`.
    pub fn new(value: &'de Value) -> Self {
        ValueDeserializer {
            value,
            path: String::new(),
        }
    }

    fn at(value: &'de Value, parent: &str, segment: &str) -> Self {
        let path = if parent.is_empty() {
            segment.to_string()
        } else {
            format!("{parent}.{segment}")
        };
        ValueDeserializer { value, path }
    }

    fn mismatch(&self, expected: &str) -> ConfigError {
        ConfigError::TypeMismatch {
            path: self.path.clone(),
            expected: expected.to_string(),
            found: self.value.type_name(),
            origin: None,
        }
    }
}

/// Deserializes a `T` from a [`Value`] tree.
pub fn from_value<'de, T: Deserialize<'de>>(value: &'de Value) -> Result<T, ConfigError> {
    T::deserialize(ValueDeserializer::new(value))
}

macro_rules! deserialize_parsed_number {
    ($($method:ident => $visit:ident: $ty:ty),* $(,)?) => {
        $(fn $method<V>(self, visitor: V) -> Result<V::Value, ConfigError>
        where
            V: Visitor<'de>,
        {
            // Env vars and CLI args often carry numbers as strings; parse
            // them when the target type asks for a number.
            if let Value::String(s) = self.value {
                match s.parse::<$ty>() {
                    Ok(parsed) => return visitor.$visit(parsed),
                    Err(_) => return Err(self.mismatch(stringify!($ty))),
                }
            }
            self.deserialize_any(visitor)
        })*
    };
}

impl<'de> Deserializer<'de> for ValueDeserializer<'de> {
    type Error = ConfigError;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, ConfigError>
    where
        V: Visitor<'de>,
    {
        let result = match self.value {
            Value::Null => visitor.visit_unit(),
            Value::Bool(b) => visitor.visit_bool(*b),
            Value::Integer(i) => visitor.visit_i64(*i),
            Value::Float(x) => visitor.visit_f64(*x),
            Value::String(s) => visitor.visit_borrowed_str(s),
            Value::Array(items) => visitor.visit_seq(SeqDeserializer {
                iter: items.iter().enumerate(),
                path: &self.path,
            }),
            Value::Table(table) => visitor.visit_map(MapDeserializer {
                iter: table.iter(),
                current: None,
                path: &self.path,
            }),
        };
        result.map_err(|e| e.with_path_context(&self.path))
    }

    deserialize_parsed_number! {
        deserialize_i8 => visit_i8: i8,
        deserialize_i16 => visit_i16: i16,
        deserialize_i32 => visit_i32: i32,
        deserialize_i64 => visit_i64: i64,
        deserialize_u8 => visit_u8: u8,
        deserialize_u16 => visit_u16: u16,
        deserialize_u32 => visit_u32: u32,
        deserialize_u64 => visit_u64: u64,
        deserialize_f32 => visit_f32: f32,
        deserialize_f64 => visit_f64: f64,
    }

    fn deserialize_bool<V>(self, visitor: V) -> Result<V::Value, ConfigError>
    where
        V: Visitor<'de>,
    {
        if let Value::String(s) = self.value {
            return match s.as_str() {
                "true" => visitor.visit_bool(true),
                "false" => visitor.visit_bool(false),
                _ => Err(self.mismatch("boolean")),
            };
        }
        self.deserialize_any(visitor)
    }

    fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, ConfigError>
    where
        V: Visitor<'de>,
    {
        // Scalars coerce to strings so a `String` field accepts `port = 8080`
        // and env-provided values symmetrically.
        match self.value {
            Value::Bool(b) => visitor.visit_string(b.to_string()),
            Value::Integer(i) => visitor.visit_string(i.to_string()),
            Value::Float(x) => visitor.visit_string(x.to_string()),
            _ => self.deserialize_any(visitor),
        }
    }

    fn deserialize_string<V>(self, visitor: V) -> Result<V::Value, ConfigError>
    where
        V: Visitor<'de>,
    {
        self.deserialize_str(visitor)
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, ConfigError>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Value::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_newtype_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, ConfigError>
    where
        V: Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_enum<V>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, ConfigError>
    where
        V: Visitor<'de>,
    {
        match self.value {
            // A bare string is a unit variant: `level = "debug"`.
            Value::String(variant) => visitor.visit_enum(EnumDeserializer {
                variant,
                value: None,
                path: &self.path,
            }),
            // A single-entry table is a variant with payload:
            // `backend = { postgres = { url = "..." } }`.
            Value::Table(table) if table.len() == 1 => {
                let (variant, value) = table.iter().next().expect("len checked");
                visitor.visit_enum(EnumDeserializer {
                    variant,
                    value: Some(value),
                    path: &self.path,
                })
            }
            _ => Err(self.mismatch("a string or single-entry table naming an enum variant")),
        }
    }

    fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value, ConfigError>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Value::Null => visitor.visit_unit(),
            _ => Err(self.mismatch("null")),
        }
    }

    fn deserialize_unit_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, ConfigError>
    where
        V: Visitor<'de>,
    {
        self.deserialize_unit(visitor)
    }

    fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value, ConfigError>
    where
        V: Visitor<'de>,
    {
        match self.value {
            // A lone scalar coerces to a single-element sequence, so an
            // env-provided `APP_TAGS=a` fills a `Vec<String>` just like
            // `APP_TAGS=a,b` does once split by a list separator.
            Value::Bool(_) | Value::Integer(_) | Value::Float(_) | Value::String(_) => visitor
                .visit_seq(SingleElementSeq {
                    value: Some(self.value),
                    path: &self.path,
                }),
            _ => self.deserialize_any(visitor),
        }
    }

    serde::forward_to_deserialize_any! {
        char bytes byte_buf tuple tuple_struct map struct identifier
        ignored_any
    }
}

/// Backs the scalar→sequence coercion in `deserialize_seq`.
struct SingleElementSeq<'de, 'p> {
    value: Option<&'de Value>,
    path: &'p str,
}

impl<'de> SeqAccess<'de> for SingleElementSeq<'de, '_> {
    type Error = ConfigError;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, ConfigError>
    where
        T: DeserializeSeed<'de>,
    {
        match self.value.take() {
            Some(value) => seed
                .deserialize(ValueDeserializer::at(value, self.path, "0"))
                .map(Some),
            None => Ok(None),
        }
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.value.is_some() as usize)
    }
}

struct SeqDeserializer<'de, 'p> {
    iter: std::iter::Enumerate<std::slice::Iter<'de, Value>>,
    path: &'p str,
}

impl<'de> SeqAccess<'de> for SeqDeserializer<'de, '_> {
    type Error = ConfigError;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, ConfigError>
    where
        T: DeserializeSeed<'de>,
    {
        match self.iter.next() {
            Some((index, value)) => seed
                .deserialize(ValueDeserializer::at(value, self.path, &index.to_string()))
                .map(Some),
            None => Ok(None),
        }
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.iter.len())
    }
}

struct MapDeserializer<'de, 'p> {
    iter: btree_map::Iter<'de, String, Value>,
    current: Option<(&'de String, &'de Value)>,
    path: &'p str,
}

impl<'de> MapAccess<'de> for MapDeserializer<'de, '_> {
    type Error = ConfigError;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, ConfigError>
    where
        K: DeserializeSeed<'de>,
    {
        match self.iter.next() {
            Some(entry) => {
                self.current = Some(entry);
                seed.deserialize(entry.0.as_str().into_deserializer())
                    .map(Some)
            }
            None => Ok(None),
        }
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, ConfigError>
    where
        V: DeserializeSeed<'de>,
    {
        let (key, value) = self
            .current
            .take()
            .expect("next_value called before next_key");
        seed.deserialize(ValueDeserializer::at(value, self.path, key))
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.iter.len())
    }
}

struct EnumDeserializer<'de, 'p> {
    variant: &'de str,
    value: Option<&'de Value>,
    path: &'p str,
}

impl<'de, 'p> EnumAccess<'de> for EnumDeserializer<'de, 'p> {
    type Error = ConfigError;
    type Variant = VariantDeserializer<'de, 'p>;

    fn variant_seed<V>(self, seed: V) -> Result<(V::Value, Self::Variant), ConfigError>
    where
        V: DeserializeSeed<'de>,
    {
        let variant = seed.deserialize(self.variant.into_deserializer())?;
        Ok((
            variant,
            VariantDeserializer {
                variant: self.variant,
                value: self.value,
                path: self.path,
            },
        ))
    }
}

struct VariantDeserializer<'de, 'p> {
    variant: &'de str,
    value: Option<&'de Value>,
    path: &'p str,
}

impl<'de> VariantAccess<'de> for VariantDeserializer<'de, '_> {
    type Error = ConfigError;

    fn unit_variant(self) -> Result<(), ConfigError> {
        match self.value {
            None | Some(Value::Null) => Ok(()),
            Some(value) => Err(ConfigError::TypeMismatch {
                path: self.path.to_string(),
                expected: format!("unit variant `{}` with no payload", self.variant),
                found: value.type_name(),
                origin: None,
            }),
        }
    }

    fn newtype_variant_seed<T>(self, seed: T) -> Result<T::Value, ConfigError>
    where
        T: DeserializeSeed<'de>,
    {
        match self.value {
            Some(value) => seed.deserialize(ValueDeserializer::at(value, self.path, self.variant)),
            None => Err(de::Error::custom(format!(
                "variant `{}` expects a payload but none was provided",
                self.variant
            ))),
        }
    }

    fn tuple_variant<V>(self, _len: usize, visitor: V) -> Result<V::Value, ConfigError>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Some(value) => {
                ValueDeserializer::at(value, self.path, self.variant).deserialize_any(visitor)
            }
            None => Err(de::Error::custom(format!(
                "variant `{}` expects a tuple payload but none was provided",
                self.variant
            ))),
        }
    }

    fn struct_variant<V>(
        self,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, ConfigError>
    where
        V: Visitor<'de>,
    {
        match self.value {
            Some(value) => {
                ValueDeserializer::at(value, self.path, self.variant).deserialize_any(visitor)
            }
            None => Err(de::Error::custom(format!(
                "variant `{}` expects a struct payload but none was provided",
                self.variant
            ))),
        }
    }
}
