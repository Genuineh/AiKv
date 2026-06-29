//! RESP 类型 golden encode 测试 (Step 1)

use aikv::protocol::{ProtocolVersion, RespValue};
use bytes::Bytes;

fn golden(value: RespValue, expected: &[u8]) {
    assert_eq!(value.serialize().as_ref(), expected);
}

#[test]
fn test_simple_string_encode() {
    golden(RespValue::SimpleString("OK".into()), b"+OK\r\n");
}

#[test]
fn test_error_encode() {
    golden(RespValue::Error("ERR msg".into()), b"-ERR msg\r\n");
}

#[test]
fn test_integer_encode_positive() {
    golden(RespValue::Integer(1000), b":1000\r\n");
}

#[test]
fn test_integer_encode_negative() {
    golden(RespValue::Integer(-42), b":-42\r\n");
}

#[test]
fn test_integer_encode_zero() {
    golden(RespValue::Integer(0), b":0\r\n");
}

#[test]
fn test_bulk_string_encode() {
    golden(
        RespValue::BulkString(Some(Bytes::from_static(b"foobar"))),
        b"$6\r\nfoobar\r\n",
    );
}

#[test]
fn test_null_bulk_string_encode() {
    golden(RespValue::BulkString(None), b"$-1\r\n");
}

#[test]
fn test_empty_bulk_string_encode() {
    golden(RespValue::BulkString(Some(Bytes::new())), b"$0\r\n\r\n");
}

#[test]
fn test_bulk_string_binary_data_encode() {
    let data = Bytes::from(vec![0x00, 0xFF, b'\r', b'\n', 0x80]);
    golden(
        RespValue::BulkString(Some(data.clone())),
        b"$5\r\n\x00\xFF\r\n\x80\r\n",
    );
}

#[test]
fn test_array_encode() {
    golden(
        RespValue::Array(Some(vec![
            RespValue::BulkString(Some(Bytes::from_static(b"foo"))),
            RespValue::BulkString(Some(Bytes::from_static(b"bar"))),
        ])),
        b"*2\r\n$3\r\nfoo\r\n$3\r\nbar\r\n",
    );
}

#[test]
fn test_null_array_encode() {
    golden(RespValue::Array(None), b"*-1\r\n");
}

#[test]
fn test_empty_array_encode() {
    golden(RespValue::Array(Some(vec![])), b"*0\r\n");
}

#[test]
fn test_resp3_null_encode() {
    golden(RespValue::Null, b"_\r\n");
}

#[test]
fn test_resp3_boolean_true_encode() {
    golden(RespValue::Boolean(true), b"#t\r\n");
}

#[test]
fn test_resp3_boolean_false_encode() {
    golden(RespValue::Boolean(false), b"#f\r\n");
}

#[test]
fn test_resp3_double_encode() {
    golden(RespValue::Double(314.0 / 100.0), b",3.14\r\n");
}

#[test]
fn test_double_inf_encode() {
    golden(RespValue::Double(f64::INFINITY), b",inf\r\n");
}

#[test]
fn test_double_neg_inf_encode() {
    golden(RespValue::Double(f64::NEG_INFINITY), b",-inf\r\n");
}

#[test]
fn test_double_nan_encode() {
    golden(RespValue::Double(f64::NAN), b",nan\r\n");
}

#[test]
fn test_double_negative_zero_encode() {
    golden(RespValue::Double(-0.0_f64), b",-0\r\n");
}

#[test]
fn test_resp3_big_number_encode() {
    golden(
        RespValue::BigNumber("3492890328409238509324850943850943825024385".into()),
        b"(3492890328409238509324850943850943825024385\r\n",
    );
}

#[test]
fn test_resp3_bulk_error_encode() {
    golden(
        RespValue::BulkError("SYNTAX invalid syntax".into()),
        b"!21\r\nSYNTAX invalid syntax\r\n",
    );
}

#[test]
fn test_resp3_verbatim_string_encode() {
    golden(
        RespValue::VerbatimString {
            format: "txt".into(),
            data: Bytes::from_static(b"Some string"),
        },
        b"=15\r\ntxt:Some string\r\n",
    );
}

#[test]
fn test_resp3_map_encode() {
    golden(
        RespValue::Map(vec![
            (
                RespValue::SimpleString("first".into()),
                RespValue::Integer(1),
            ),
            (
                RespValue::SimpleString("second".into()),
                RespValue::Integer(2),
            ),
        ]),
        b"%2\r\n+first\r\n:1\r\n+second\r\n:2\r\n",
    );
}

#[test]
fn test_resp3_set_encode() {
    golden(
        RespValue::Set(vec![
            RespValue::SimpleString("orange".into()),
            RespValue::SimpleString("apple".into()),
        ]),
        b"~2\r\n+orange\r\n+apple\r\n",
    );
}

#[test]
fn test_resp3_push_encode() {
    golden(
        RespValue::Push(vec![
            RespValue::SimpleString("pubsub".into()),
            RespValue::SimpleString("message".into()),
        ]),
        b">2\r\n+pubsub\r\n+message\r\n",
    );
}

#[test]
fn test_resp3_attribute_encode() {
    golden(
        RespValue::Attribute {
            attributes: vec![(
                RespValue::SimpleString("key".into()),
                RespValue::Integer(3600),
            )],
            data: Box::new(RespValue::SimpleString("OK".into())),
        },
        b"|1\r\n+key\r\n:3600\r\n+OK\r\n",
    );
}

#[test]
fn test_resp3_streamed_string_encode() {
    golden(
        RespValue::StreamedString(vec![Bytes::from_static(b"Hell")]),
        b"$?\r\n;4\r\nHell\r\n;0\r\n",
    );
}

#[test]
fn test_protocol_version_default() {
    // 默认协议版本为 RESP3，新连接无需 HELLO 3 即可使用 RESP3 类型
    assert_eq!(ProtocolVersion::default(), ProtocolVersion::Resp3);
    assert_eq!(ProtocolVersion::Resp2.as_u8(), 2);
    assert_eq!(ProtocolVersion::Resp3.as_u8(), 3);
}
