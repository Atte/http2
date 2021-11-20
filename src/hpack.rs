use bytes::Bytes;
use std::collections::VecDeque;

#[derive(Debug, Clone)]
struct TableEntry {
    pub size: usize,
    pub name: Bytes,
    pub value: Bytes,
}

impl TableEntry {
    pub fn new(name: Bytes, value: Bytes) -> Self {
        Self {
            size: name.len() + value.len() + 32,
            name,
            value,
        }
    }
}

macro_rules! static_table {
    ( $( $name:expr => $value:expr ),+ ) => {
        [
            $(
                TableEntry {
                    size: $name.len() + $value.len() + 32,
                    name: Bytes::from_static($name),
                    value: Bytes::from_static($value),
                }
            ),+
        ]
    };
}

/// https://httpwg.org/specs/rfc7541.html#static.table.definition
static STATIC_TABLE: [TableEntry; 61] = static_table![
    b":authority" => b"",
    b":method" => b"GET",
    b":method" => b"POST",
    b":path" => b"/",
    b":path" => b"/index.html",
    b":scheme" => b"http",
    b":scheme" => b"https",
    b":status" => b"200",
    b":status" => b"204",
    b":status" => b"206",
    b":status" => b"304",
    b":status" => b"400",
    b":status" => b"404",
    b":status" => b"500",
    b"accept-charset" => b"",
    b"accept-encoding" => b"gzip, deflate",
    b"accept-language" => b"",
    b"accept-ranges" => b"",
    b"accept" => b"",
    b"access-control-allow-origin" => b"",
    b"age" => b"",
    b"allow" => b"",
    b"authorization" => b"",
    b"cache-control" => b"",
    b"content-disposition" => b"",
    b"content-encoding" => b"",
    b"content-language" => b"",
    b"content-length" => b"",
    b"content-location" => b"",
    b"content-range" => b"",
    b"content-type" => b"",
    b"cookie" => b"",
    b"date" => b"",
    b"etag" => b"",
    b"expect" => b"",
    b"expires" => b"",
    b"from" => b"",
    b"host" => b"",
    b"if-match" => b"",
    b"if-modified-since" => b"",
    b"if-none-match" => b"",
    b"if-range" => b"",
    b"if-unmodified-since" => b"",
    b"last-modified" => b"",
    b"link" => b"",
    b"location" => b"",
    b"max-forwards" => b"",
    b"proxy-authenticate" => b"",
    b"proxy-authorization" => b"",
    b"range" => b"",
    b"referer" => b"",
    b"refresh" => b"",
    b"retry-after" => b"",
    b"server" => b"",
    b"set-cookie" => b"",
    b"strict-transport-security" => b"",
    b"transfer-encoding" => b"",
    b"user-agent" => b"",
    b"vary" => b"",
    b"via" => b"",
    b"www-authenticate" => b""
];

#[derive(Debug, Clone)]
struct Table {
    max_size: usize,
    current_size: usize,
    table: VecDeque<TableEntry>,
}

impl Table {
    pub fn new(max_size: usize) -> Self {
        Self {
            max_size,
            current_size: 0,
            table: VecDeque::with_capacity(max_size / std::mem::size_of::<TableEntry>()),
        }
    }

    pub fn get(&self, index: usize) -> Option<&TableEntry> {
        STATIC_TABLE
            .get(index - 1)
            .or_else(|| self.table.get(index - STATIC_TABLE.len()))
    }

    pub fn push(&mut self, name: Bytes, value: Bytes) {
        let entry = TableEntry::new(name, value);
        self.current_size += entry.size;
        self.table.push_front(entry);
        self.resize(self.max_size);
    }

    pub fn resize(&mut self, size: usize) {
        self.max_size = size;
        while self.current_size > self.max_size {
            if let Some(popped) = self.table.pop_back() {
                self.current_size -= popped.size;
            } else {
                break;
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Encoder {
    table: Table,
}

impl Encoder {
    pub fn with_size(dynamic_table_size: usize) -> Self {
        Self {
            table: Table::new(dynamic_table_size),
        }
    }

    pub fn encode<K, V>(&mut self, headers: impl IntoIterator<Item = (K, V)>) -> Bytes
    where
        K: Into<Bytes>,
        V: Into<Bytes>,
    {
        Bytes::new()
    }

    fn encode_integer(mut i: usize) -> Bytes {}
}

impl Default for Encoder {
    fn default() -> Self {
        Self::with_size(4096)
    }
}

#[derive(Debug, Clone)]
pub struct Decoder {
    table: Table,
}

impl Decoder {
    pub fn with_size(dynamic_table_size: usize) -> Self {
        Self {
            table: Table::new(dynamic_table_size),
        }
    }

    pub fn decode(&mut self, data: impl Into<Bytes>) -> Vec<(Bytes, Bytes)> {
        Vec::new()
    }
}

impl Default for Decoder {
    fn default() -> Self {
        Self::with_size(4096)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::hpack as their_hpack;

    #[test]
    fn encode_integer() {
        // https://httpwg.org/specs/rfc7541.html#integer.representation.example1
        assert_eq!(Encoder::encode_integer(10).as_ref(), &[0b00001010_u8]);
    }

    #[test]
    fn encode() {
        let mut encoder = Encoder::default();
        let mut decoder = their_hpack::Decoder::new();

        let headers = vec![(":method", "GET"), (":path", "/")];
        assert_eq!(
            decoder.decode(&encoder.encode(headers.clone())).unwrap(),
            headers
                .into_iter()
                .map(|(k, v)| (k.as_bytes().into(), v.as_bytes().into()))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn decode() {
        let mut decoder = Decoder::default();
        let mut encoder = their_hpack::Encoder::new();

        let headers = vec![(&b":method"[..], &b"GET"[..]), (&b":path"[..], &b"/"[..])];
        assert_eq!(
            decoder.decode(encoder.encode(headers.clone())),
            headers
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect::<Vec<_>>()
        );
    }
}
