//! RESP Parser roundtrip 与边界测试 (Step 2)

use bytes::Bytes;
use aikv::protocol::{RespParser, RespValue};

fn roundtrip(value: RespValue) {
  let encoded = value.serialize();
  let mut parser = RespParser::new();
  parser.feed(&encoded);
  let parsed = parser.parse().expect("parse error").expect("incomplete");
  assert_eq!(parsed, value);
  assert_eq!(parser.buffer_len(), 0);
}

fn roundtrip_f64(original: f64, parsed: f64) {
  assert!(original.is_nan() && parsed.is_nan() || (original - parsed).abs() < f64::EPSILON);
}

#[test]
fn test_simple_string_roundtrip() {
  roundtrip(RespValue::SimpleString("OK".into()));
}

#[test]
fn test_error_roundtrip() {
  roundtrip(RespValue::Error("ERR msg".into()));
}

#[test]
fn test_integer_roundtrip() {
  roundtrip(RespValue::Integer(1000));
  roundtrip(RespValue::Integer(-42));
  roundtrip(RespValue::Integer(0));
  roundtrip(RespValue::Integer(i64::MAX));
  roundtrip(RespValue::Integer(i64::MIN));
}

#[test]
fn test_bulk_string_roundtrip() {
  roundtrip(RespValue::BulkString(Some(Bytes::from_static(b"foobar"))));
  roundtrip(RespValue::BulkString(Some(Bytes::new())));
}

#[test]
fn test_null_bulk_string_roundtrip() {
  roundtrip(RespValue::BulkString(None));
}

#[test]
fn test_array_roundtrip() {
  roundtrip(RespValue::Array(Some(vec![
    RespValue::BulkString(Some(Bytes::from_static(b"foo"))),
    RespValue::BulkString(Some(Bytes::from_static(b"bar"))),
  ])));
  roundtrip(RespValue::Array(Some(vec![])));
  roundtrip(RespValue::Array(Some(vec![RespValue::Array(Some(vec![
    RespValue::Integer(1),
  ]))])));
}

#[test]
fn test_null_array_roundtrip() {
  roundtrip(RespValue::Array(None));
}

#[test]
fn test_bulk_string_binary_data() {
  let data = Bytes::from(vec![0x00, 0xFF, b'\r', b'\n', 0x80]);
  roundtrip(RespValue::BulkString(Some(data)));
}

#[test]
fn test_resp3_null_roundtrip() {
  roundtrip(RespValue::Null);
}

#[test]
fn test_resp3_boolean_roundtrip() {
  roundtrip(RespValue::Boolean(true));
  roundtrip(RespValue::Boolean(false));
}

#[test]
fn test_resp3_double_roundtrip() {
  roundtrip(RespValue::Double(314.0 / 100.0));
  roundtrip(RespValue::Double(f64::INFINITY));
  roundtrip(RespValue::Double(f64::NEG_INFINITY));
}

#[test]
fn test_double_nan_roundtrip() {
  let encoded = RespValue::Double(f64::NAN).serialize();
  let mut parser = RespParser::new();
  parser.feed(&encoded);
  let parsed = parser.parse().unwrap().unwrap();
  match parsed {
    RespValue::Double(d) => assert!(d.is_nan()),
    _ => panic!("expected double"),
  }
}

#[test]
fn test_double_negative_zero() {
  roundtrip(RespValue::Double(-0.0_f64));
}

#[test]
fn test_resp3_big_number_roundtrip() {
  roundtrip(RespValue::BigNumber(
    "3492890328409238509324850943850943825024385".into(),
  ));
}

#[test]
fn test_resp3_bulk_error_roundtrip() {
  roundtrip(RespValue::BulkError("SYNTAX invalid syntax".into()));
}

#[test]
fn test_resp3_map_roundtrip() {
  roundtrip(RespValue::Map(vec![
    (
      RespValue::SimpleString("first".into()),
      RespValue::Integer(1),
    ),
    (
      RespValue::SimpleString("second".into()),
      RespValue::Integer(2),
    ),
  ]));
}

#[test]
fn test_resp3_set_roundtrip() {
  roundtrip(RespValue::Set(vec![
    RespValue::SimpleString("orange".into()),
    RespValue::SimpleString("apple".into()),
  ]));
}

#[test]
fn test_resp3_push_roundtrip() {
  roundtrip(RespValue::Push(vec![
    RespValue::SimpleString("pubsub".into()),
    RespValue::SimpleString("message".into()),
  ]));
}

#[test]
fn test_resp3_attribute_roundtrip() {
  roundtrip(RespValue::Attribute {
    attributes: vec![(
      RespValue::SimpleString("key".into()),
      RespValue::Integer(3600),
    )],
    data: Box::new(RespValue::SimpleString("OK".into())),
  });
}

#[test]
fn test_resp3_verbatim_string_roundtrip() {
  roundtrip(RespValue::VerbatimString {
    format: "txt".into(),
    data: Bytes::from_static(b"Some string"),
  });
}

#[test]
fn test_resp3_streamed_string_roundtrip() {
  roundtrip(RespValue::StreamedString(vec![Bytes::from_static(b"Hell")]));
  roundtrip(RespValue::StreamedString(vec![]));
}

#[test]
fn test_parse_incomplete_data() {
  let mut parser = RespParser::new();
  parser.feed(b"+OK");
  assert!(parser.parse().unwrap().is_none());
  assert_eq!(parser.buffer_len(), 3);
  parser.feed(b"\r\n");
  assert_eq!(
    parser.parse().unwrap(),
    Some(RespValue::SimpleString("OK".into()))
  );
}

#[test]
fn test_parse_pipeline() {
  let mut parser = RespParser::new();
  parser.feed(b"+PONG\r\n+PONG\r\n");
  assert_eq!(
    parser.parse().unwrap(),
    Some(RespValue::SimpleString("PONG".into()))
  );
  assert_eq!(
    parser.parse().unwrap(),
    Some(RespValue::SimpleString("PONG".into()))
  );
  assert!(parser.parse().unwrap().is_none());
}

#[test]
fn test_empty_array_in_pipeline() {
  let mut parser = RespParser::new();
  parser.feed(b"*0\r\n+PONG\r\n");
  assert_eq!(
    parser.parse().unwrap(),
    Some(RespValue::Array(Some(vec![])))
  );
  assert_eq!(
    parser.parse().unwrap(),
    Some(RespValue::SimpleString("PONG".into()))
  );
}

#[test]
fn test_parse_negative_length_invalid() {
  let mut p1 = RespParser::new();
  p1.feed(b"$-2\r\n");
  assert!(p1.parse().is_err());

  let mut p2 = RespParser::new();
  p2.feed(b"%-1\r\n");
  assert!(p2.parse().is_err());
}

#[test]
fn test_parse_length_overflow() {
  let mut parser = RespParser::new();
  parser
    .feed(b":9999999999999999999999999999999999999999999999999999999999999999999999999999999\r\n");
  assert!(parser.parse().is_err());
}

#[test]
fn test_parse_empty_map_set_push() {
  roundtrip(RespValue::Map(vec![]));
  roundtrip(RespValue::Set(vec![]));
  roundtrip(RespValue::Push(vec![]));
}

#[test]
fn test_parse_unknown_type_marker() {
  let mut parser = RespParser::new();
  parser.feed(b"\xFFbad\r\n+OK\r\n");
  let got = loop {
    match parser.parse() {
      Err(_) => continue,
      Ok(Some(v)) => break v,
      Ok(None) => panic!("unexpected incomplete frame"),
    }
  };
  assert_eq!(got, RespValue::SimpleString("OK".into()));
}

#[test]
fn test_parse_depth_limit() {
  let mut parser = RespParser::with_limits(1024, 1024 * 1024, 2, 1024, 1024);
  // depth 0: array, depth 1: array, depth 2: array -> exceeds depth 2
  parser.feed(b"*1\r\n*1\r\n*1\r\n:1\r\n");
  assert!(parser.parse().is_err());
}

#[test]
fn test_parse_bulk_string_too_large() {
  let mut parser = RespParser::with_limits(4, 1024, 128, 1024, 1024);
  parser.feed(b"$5\r\nhello\r\n");
  assert!(parser.parse().is_err());
}

#[test]
fn test_parse_array_too_large() {
  let mut parser = RespParser::with_limits(1024, 1024 * 1024, 128, 2, 1024);
  parser.feed(b"*3\r\n:1\r\n:2\r\n:3\r\n");
  assert!(parser.parse().is_err());
}

#[test]
fn test_parse_map_too_large() {
  let mut parser = RespParser::with_limits(1024, 1024 * 1024, 128, 1, 1024);
  parser.feed(b"%2\r\n+first\r\n:1\r\n+second\r\n:2\r\n");
  assert!(parser.parse().is_err());
}

#[test]
fn test_parse_line_protocol_too_long() {
  let mut parser = RespParser::with_limits(1024, 1024 * 1024, 128, 1024, 4);
  parser.feed(b"+hello\r\n");
  assert!(parser.parse().is_err());
}

#[test]
fn test_parse_big_number_too_long() {
  let mut parser = RespParser::with_limits(1024, 1024 * 1024, 128, 1024, 4);
  parser.feed(b"(12345\r\n");
  assert!(parser.parse().is_err());
}

#[test]
fn test_parse_length_mismatch() {
  let mut parser = RespParser::new();
  parser.feed(b"$3\r\nabcd\r\n");
  assert!(parser.parse().is_err());
}

#[test]
fn test_verbatim_string_malformed() {
  let mut parser = RespParser::new();
  parser.feed(b"=5\r\nab:cd\r\n");
  assert!(parser.parse().is_err());
}

#[test]
fn test_integer_overflow() {
  let mut parser = RespParser::new();
  parser
    .feed(b":9999999999999999999999999999999999999999999999999999999999999999999999999999999\r\n");
  assert!(parser.parse().is_err());
}

// suppress unused warning for roundtrip_f64 helper reserved for future use
#[allow(dead_code)]
fn _keep_roundtrip_f64() {
  roundtrip_f64(1.0, 1.0);
}
