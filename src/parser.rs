use std::collections::HashMap;

use nom::{
    IResult, Parser,
    bytes::complete::{tag, take_until},
    character::complete::{char, space0},
    combinator::{map, verify},
    multi::many1,
    sequence::{separated_pair, terminated},
};

#[inline]
fn key_name(input: &[u8]) -> IResult<&[u8], &[u8]> {
    verify(take_until(":"), |input: &[u8]| {
        if !input.is_empty() {
            input[0] != b'\n'
        } else {
            false
        }
    })
    .parse(input)
}

#[inline]
fn separator(input: &[u8]) -> IResult<&[u8], ()> {
    map((char(':'), space0), |_| ()).parse(input)
}

#[inline]
fn single_line(input: &[u8]) -> IResult<&[u8], &[u8]> {
    take_until("\n")(input)
}

#[inline]
fn key_value(input: &[u8]) -> IResult<&[u8], (&[u8], &[u8])> {
    separated_pair(key_name, separator, single_line).parse(input)
}

#[inline]
fn single_package(input: &[u8]) -> IResult<&[u8], Vec<(&[u8], &[u8])>> {
    many1(terminated(key_value, tag("\n"))).parse(input)
}

#[inline]
#[allow(clippy::type_complexity)]
pub fn single_package_map(input: &[u8]) -> IResult<&[u8], HashMap<&[u8], &[u8]>> {
    let mut map = HashMap::new();
    let (ret, res) = single_package(input)?;

    for (key, value) in res {
        map.insert(key, value);
    }

    Ok((ret, map))
}

#[test]
fn test_key_name() {
    let test = &b"name: value"[..];
    assert_eq!(key_name(test), Ok((&b": value"[..], &b"name"[..])));
}

#[test]
fn test_seperator() {
    let test = &b": value"[..];
    let test_2 = &b": \tvalue"[..];
    assert_eq!(separator(test), Ok((&b"value"[..], ())));
    assert_eq!(separator(test_2), Ok((&b"value"[..], ())));
}

#[test]
fn test_single_line() {
    let test = &b"value\n"[..];
    let test_2 = &b"value\t\r\n"[..];
    let test_3 = &b"value \x23\xff\n"[..];
    assert_eq!(single_line(test), Ok((&b"\n"[..], &b"value"[..])));
    assert_eq!(single_line(test_2), Ok((&b"\n"[..], &b"value\t\r"[..])));
    assert_eq!(
        single_line(test_3),
        Ok((&b"\n"[..], &b"value \x23\xff"[..]))
    );
}

#[test]
fn test_key_value() {
    let test = &b"name1: value\n"[..];
    let test_2 = &b"name2: value\t\r\n"[..];
    let test_3 = &b"name3: value \x23\xff\n"[..];
    assert_eq!(
        key_value(test),
        Ok((&b"\n"[..], (&b"name1"[..], &b"value"[..])))
    );
    assert_eq!(
        key_value(test_2),
        Ok((&b"\n"[..], (&b"name2"[..], &b"value\t\r"[..])))
    );
    assert_eq!(
        key_value(test_3),
        Ok((&b"\n"[..], (&b"name3"[..], &b"value \x23\xff"[..])))
    );
}

#[test]
fn test_package() {
    let test = &b"Package: zsync\nVersion: 0.6.2-1\nSection: net\nArchitecture: amd64\nInstalled-Size: 256\n\n"[..];
    assert_eq!(
        single_package(test),
        Ok((
            &b"\n"[..],
            vec![
                (&b"Package"[..], &b"zsync"[..]),
                (&b"Version"[..], &b"0.6.2-1"[..]),
                (&b"Section"[..], &b"net"[..]),
                (&b"Architecture"[..], &b"amd64"[..]),
                (&b"Installed-Size"[..], &b"256"[..])
            ]
        ))
    );
}
