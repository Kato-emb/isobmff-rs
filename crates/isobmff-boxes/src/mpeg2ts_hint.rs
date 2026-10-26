//! The boxes of the additional data of an MPEG-2 TS hint sample entry: the PSI tables of the stream and the timing of its samples

use alloc::vec::Vec;

use isobmff_core::{BoxDecode, BoxDefinition, BoxEncode, BoxType, Error, FieldReader, FieldWriter};

/// Largest `PID`, which takes 13 bits
const PID_MAX: u16 = 0x1fff;

/// Reads the 13-bit `PID` after three `reserved` bits and the section data to the end of the payload
fn read_table(reader: &mut FieldReader<'_>) -> Result<(u16, Vec<u8>), Error> {
    let pid = reader.read_u16()? & PID_MAX;

    Ok((pid, reader.take_remainder().to_vec()))
}

/// Writes the fields [`read_table`] reads, the `reserved` bits as zero
///
/// # Errors
///
/// * [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for `PID`:
///   `pid` is wider than its 13 bits.
fn write_table(writer: &mut FieldWriter<'_>, pid: u16, section_data: &[u8]) -> Result<(), Error> {
    if pid > PID_MAX {
        return Err(Error::invalid_value("PID"));
    }
    writer.write_u16(pid)?;
    writer.write_slice(section_data)
}

/// Returns the bytes the payload of a table of `section_data` takes
fn section_table_len(section_data: &[u8]) -> u64 {
    u64::try_from(section_data.len())
        .unwrap_or(u64::MAX)
        .saturating_add(2)
}

/// [`PatBox`] (`tPAT`), ISO/IEC 14496-12 §9.3.3.3
///
/// The program association table (PAT) of the MPEG-2 transport stream an
/// MPEG-2 TS hint sample entry describes, as the concatenated sections of
/// one version, kept as found.
///
/// The three `reserved` bits are read past and written as zero. A `pid`
/// wider than its 13 bits is refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for `PID` when
/// the box is written.
#[doc(alias = "tPAT", alias = "PATBox")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct PatBox {
    /// PID of the packets the table was extracted from, always 0
    pub pid: u16,
    /// Sections of the table
    pub section_data: Vec<u8>,
}

impl BoxDefinition for PatBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"tPAT");
}

impl BoxDecode for PatBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let (pid, section_data) = read_table(reader)?;

        Ok(Self { pid, section_data })
    }
}

impl BoxEncode for PatBox {
    fn payload_len(&self) -> u64 {
        section_table_len(&self.section_data)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_table(writer, self.pid, &self.section_data)
    }
}

/// [`PmtBox`] (`tPMT`), ISO/IEC 14496-12 §9.3.3.3
///
/// The program map table (PMT) of one program of the MPEG-2 transport stream
/// an MPEG-2 TS hint sample entry describes, as [`PatBox`] holds the PAT.
#[doc(alias = "tPMT", alias = "PMTBox")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct PmtBox {
    /// PID of the packets the table was extracted from
    pub pid: u16,
    /// Sections of the table
    pub section_data: Vec<u8>,
}

impl BoxDefinition for PmtBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"tPMT");
}

impl BoxDecode for PmtBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let (pid, section_data) = read_table(reader)?;

        Ok(Self { pid, section_data })
    }
}

impl BoxEncode for PmtBox {
    fn payload_len(&self) -> u64 {
        section_table_len(&self.section_data)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_table(writer, self.pid, &self.section_data)
    }
}

/// [`OdBox`] (`tOD `), ISO/IEC 14496-12 §9.3.3.3
///
/// An object descriptor table of the MPEG-2 transport stream an MPEG-2 TS
/// hint sample entry describes, as [`PatBox`] holds the PAT.
#[doc(alias = "tOD", alias = "ODBox")]
#[non_exhaustive]
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct OdBox {
    /// PID of the packets the table was extracted from
    pub pid: u16,
    /// Sections of the table
    pub section_data: Vec<u8>,
}

impl BoxDefinition for OdBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"tOD ");
}

impl BoxDecode for OdBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let (pid, section_data) = read_table(reader)?;

        Ok(Self { pid, section_data })
    }
}

impl BoxEncode for OdBox {
    fn payload_len(&self) -> u64 {
        section_table_len(&self.section_data)
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        write_table(writer, self.pid, &self.section_data)
    }
}

/// [`TsTimingBox`] (`tsti`), ISO/IEC 14496-12 §9.3.3.3
///
/// How the sample times of the packets of a PID were derived: from the
/// reception time (0), or by piecewise linearity between PCRs (1).
///
/// The two `reserved` bits are read past and written as zero. A
/// `timing_derivation_method` wider than its bit, or a `pid` wider than its
/// 13 bits, is refused with
/// [`InvalidValue`](isobmff_core::ErrorKind::InvalidValue) for the field when
/// the box is written.
#[doc(alias = "tsti", alias = "TSTimingBox")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct TsTimingBox {
    /// Method the sample times were derived by, 0 or 1
    pub timing_derivation_method: u8,
    /// PID the method applies to
    pub pid: u16,
}

impl BoxDefinition for TsTimingBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"tsti");
}

impl BoxDecode for TsTimingBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let fields = reader.read_u16()?;

        Ok(Self {
            timing_derivation_method: u8::from(fields & 0x8000 != 0),
            pid: fields & PID_MAX,
        })
    }
}

impl BoxEncode for TsTimingBox {
    fn payload_len(&self) -> u64 {
        2
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        let method = match self.timing_derivation_method {
            0 => 0,
            1 => 0x8000,
            _ => return Err(Error::invalid_value("timing_derivation_method")),
        };
        if self.pid > PID_MAX {
            return Err(Error::invalid_value("PID"));
        }

        writer.write_u16(method | self.pid)
    }
}

/// [`InitialSampleTimeBox`] (`istm`), ISO/IEC 14496-12 §9.3.3.3
///
/// The initial value of the sample times of an MPEG-2 TS hint track, which
/// does not start from 0, from which the original sample times are
/// reconstructed. The 32 `reserved` bits are read past and written as zero.
#[doc(alias = "istm")]
#[non_exhaustive]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct InitialSampleTimeBox {
    /// Time the sample times start from
    pub initial_sample_time: u32,
}

impl BoxDefinition for InitialSampleTimeBox {
    const BOX_TYPE: BoxType = BoxType::compact(*b"istm");
}

impl BoxDecode for InitialSampleTimeBox {
    fn decode_fields(reader: &mut FieldReader<'_>) -> Result<Self, Error> {
        let initial_sample_time = reader.read_u32()?;
        reader.read_bytes::<4>()?;

        Ok(Self {
            initial_sample_time,
        })
    }
}

impl BoxEncode for InitialSampleTimeBox {
    fn payload_len(&self) -> u64 {
        4 + 4
    }

    fn encode_fields(&self, writer: &mut FieldWriter<'_>) -> Result<(), Error> {
        writer.write_u32(self.initial_sample_time)?;
        writer.write_bytes(&[0; 4])
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use isobmff_core::{BoxDecode, BoxEncode, Error};

    use super::{InitialSampleTimeBox, OdBox, PatBox, PmtBox, TsTimingBox};
    use crate::encoded;

    #[test]
    fn the_tables_read_their_pid_past_the_reserved_bits_and_keep_their_sections() {
        let (pat, _) = PatBox::decode(b"\0\0\0\x0dtPAT\xe0\0\0\xb0\x0d").unwrap();
        let (pmt, _) = PmtBox::decode(b"\0\0\0\x0ctPMT\xe1\0\x02\xb0").unwrap();
        let (od, _) = OdBox::decode(b"\0\0\0\x0btOD \x00\x65\x4f").unwrap();

        assert_eq!(
            (pat.clone(), pmt.clone(), od.clone()),
            (
                PatBox {
                    pid: 0,
                    section_data: vec![0, 0xb0, 0x0d],
                },
                PmtBox {
                    pid: 0x100,
                    section_data: vec![2, 0xb0],
                },
                OdBox {
                    pid: 0x65,
                    section_data: vec![0x4f],
                }
            )
        );
        assert_eq!(encoded(&pat), b"\0\0\0\x0dtPAT\0\0\0\xb0\x0d");
        assert_eq!(encoded(&pmt), b"\0\0\0\x0ctPMT\x01\0\x02\xb0");
        assert_eq!(encoded(&od), b"\0\0\0\x0btOD \x00\x65\x4f");
    }

    #[test]
    fn a_pid_wider_than_its_bits_is_refused_when_written() {
        let pmt = PmtBox {
            pid: 0x2000,
            section_data: vec![],
        };

        assert_eq!(
            pmt.encode(&mut [0; 10]).map(|_| ()),
            Err(Error::invalid_value("PID"))
        );
    }

    #[test]
    fn the_timing_reads_and_writes_its_method_and_pid() {
        let (timing, _) = TsTimingBox::decode(b"\0\0\0\x0atsti\xe1\x00").unwrap();

        assert_eq!(
            timing,
            TsTimingBox {
                timing_derivation_method: 1,
                pid: 0x100,
            }
        );
        assert_eq!(encoded(&timing), b"\0\0\0\x0atsti\x81\x00");
        assert_eq!(
            TsTimingBox {
                timing_derivation_method: 2,
                pid: 0,
            }
            .encode(&mut [0; 10])
            .map(|_| ()),
            Err(Error::invalid_value("timing_derivation_method"))
        );
    }

    #[test]
    fn the_initial_sample_time_reads_past_its_reserved_bits_and_writes_them_as_zero() {
        let (initial, _) =
            InitialSampleTimeBox::decode(b"\0\0\0\x10istm\0\x01\0\0\xff\xff\xff\xff").unwrap();

        assert_eq!(
            (encoded(&initial), initial),
            (
                b"\0\0\0\x10istm\0\x01\0\0\0\0\0\0".to_vec(),
                InitialSampleTimeBox {
                    initial_sample_time: 0x1_0000,
                }
            )
        );
    }
}
