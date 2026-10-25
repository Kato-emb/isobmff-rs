//! [`Utf8List`], a null-terminated list of space-separated UTF-8 strings

use alloc::string::String;
use core::fmt;

use super::utf8_string::Utf8String;
use crate::error::Error;

/// Field of the basic type `utf8list`, ISO/IEC 14496-12 §4.2.1
///
/// The specification defines the type as a list of UTF-8 strings separated by
/// spaces and ended by a NUL, which shall be present. The value holds the
/// bytes before the terminator as found, spaces included, so writing it back
/// gives the bytes it was read from: runs of spaces are not collapsed.
/// [`iter`](Self::iter) returns the strings the spaces separate.
///
/// Reading and writing follow [`Utf8String`]: reading accepts a list whose
/// terminator is left off and bytes that are not UTF-8, and writing writes the
/// bytes and a terminator after them only where the bytes are UTF-8.
///
/// # Examples
///
/// ```
/// use isobmff_core::Utf8List;
///
/// // A list of two namespaces
/// let namespaces = Utf8List::from_slice(b"urn:a  urn:b\0");
/// assert_eq!(
///     namespaces.iter().unwrap().collect::<Vec<_>>(),
///     ["urn:a", "urn:b"]
/// );
///
/// // Writing gives back the bytes read, both spaces included
/// let mut buffer = vec![0xff; 13];
/// assert!(namespaces.encode(&mut buffer).unwrap().is_empty());
/// assert_eq!(buffer, b"urn:a  urn:b\0");
///
/// // A list built from its text, which may not hold a NUL
/// assert_eq!(Utf8List::new(String::from("urn:a  urn:b")), Some(namespaces));
/// assert_eq!(Utf8List::new(String::from("urn:a\0urn:b")), None);
/// ```
#[doc(alias = "utf8list")]
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct Utf8List(Utf8String);

impl Utf8List {
    /// Creates the field from the text it carries, the strings of the list separated by spaces
    ///
    /// Returns `None` when `value` holds a NUL, which the terminator written
    /// after it could not be told apart from. The field that carries no
    /// strings is [`default`](Self::default): one NUL byte on the wire.
    #[must_use]
    pub fn new(value: String) -> Option<Self> {
        Utf8String::new(value).map(Self)
    }

    /// Returns the strings of the list, or `None` where its bytes are not UTF-8
    ///
    /// The strings are the runs of the text that hold no space. Leading,
    /// trailing and repeated spaces separate no empty string.
    #[must_use]
    pub fn iter(&self) -> Option<impl Iterator<Item = &str>> {
        self.as_str()
            .map(|text| text.split(' ').filter(|element| !element.is_empty()))
    }

    /// Returns the text the field carries, terminator excluded, or `None` where its bytes are not UTF-8
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        self.0.as_str()
    }

    /// Returns the bytes the field carries, terminator excluded
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// Reads the field that occupies the whole of `bytes`
    ///
    /// The field runs to the first NUL, or to the end of `bytes` where there is
    /// none. Bytes after a terminator are dropped: this reads a field that is
    /// the last of its box, so nothing that follows lays claim to them.
    #[must_use]
    pub fn from_slice(bytes: &[u8]) -> Self {
        Self(Utf8String::from_slice(bytes))
    }

    /// Returns the length the field occupies, terminator included
    #[must_use]
    pub fn encoded_len(&self) -> u64 {
        self.0.encoded_len()
    }

    /// Writes the field and its terminator into the front of `buffer` and returns what is left
    ///
    /// # Errors
    ///
    /// * [`InvalidUtf8String`](crate::ErrorKind::InvalidUtf8String): the bytes
    ///   the field carries are not UTF-8.
    /// * [`TruncatedBuffer`](crate::ErrorKind::TruncatedBuffer): `buffer` is shorter
    ///   than [`encoded_len`](Self::encoded_len).
    pub fn encode<'buffer>(&self, buffer: &'buffer mut [u8]) -> Result<&'buffer mut [u8], Error> {
        self.0.encode(buffer)
    }
}

impl fmt::Debug for Utf8List {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Utf8List(\"{}\")",
            self.as_bytes().escape_ascii()
        )
    }
}

#[cfg(test)]
mod tests {
    use alloc::format;
    use alloc::vec::Vec;

    use super::Utf8List;
    use crate::error::Error;

    fn strings(list: &Utf8List) -> Vec<&str> {
        list.iter().unwrap().collect()
    }

    #[test]
    fn a_list_of_one_string_holds_that_string() {
        let list = Utf8List::from_slice(b"urn:a\0");

        assert_eq!(strings(&list), ["urn:a"]);
    }

    #[test]
    fn a_list_of_several_strings_holds_them_in_order() {
        let list = Utf8List::from_slice(b"urn:a urn:b urn:c\0");

        assert_eq!(strings(&list), ["urn:a", "urn:b", "urn:c"]);
    }

    #[test]
    fn a_field_holding_only_a_terminator_holds_no_strings() {
        let list = Utf8List::from_slice(b"\0");

        assert_eq!(list, Utf8List::default());
        assert!(strings(&list).is_empty());
    }

    #[test]
    fn spaces_around_and_between_strings_separate_no_empty_string() {
        let list = Utf8List::from_slice(b"  urn:a   urn:b \0");

        assert_eq!(strings(&list), ["urn:a", "urn:b"]);
    }

    #[test]
    fn a_list_without_its_terminator_runs_to_the_end_of_the_field() {
        assert_eq!(
            Utf8List::from_slice(b"urn:a urn:b"),
            Utf8List::from_slice(b"urn:a urn:b\0")
        );
    }

    #[test]
    fn a_list_is_written_back_as_the_bytes_it_was_read_from() {
        let read = b"  urn:a   urn:b \0";
        let list = Utf8List::from_slice(read);
        let mut buffer = [0xff; 17];

        let rest = list.encode(&mut buffer).unwrap();

        assert!(rest.is_empty());
        assert_eq!(list.encoded_len(), 17);
        assert_eq!(&buffer, read);
    }

    #[test]
    fn a_list_that_is_not_utf8_holds_no_strings_and_is_refused_on_writing() {
        let list = Utf8List::from_slice(b"urn:\xff\0");

        assert!(list.iter().is_none());
        assert_eq!(list.as_bytes(), b"urn:\xff");
        assert_eq!(list.encode(&mut [0; 6]), Err(Error::invalid_utf8_string()));
    }

    #[test]
    fn debug_shows_the_bytes_escaped_as_ascii() {
        let list = Utf8List::from_slice(b"urn:a \x9f");

        assert_eq!(format!("{list:?}"), r#"Utf8List("urn:a \x9f")"#);
    }
}
