//! Contains Error type definitions and
//! all the functions that can only be used to decode an image

use ::std::io::{Read, Seek, SeekFrom};
use ::seek_bufread::BufReader as SeekBufRead;
use ::byteorder::{LittleEndian, ReadBytesExt};
use ::bit_field::BitField;
use ::smallvec::SmallVec;

use ::file::*;
use ::attributes::*;
use ::blocks::*;



pub type Result<T> = ::std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    NotEXR,
    Invalid(&'static str),
    Missing(&'static str),
    UnknownAttributeType { bytes_to_skip: u32 },

    IoError(::std::io::Error),
    CompressionError(::compress::Error),

    NotSupported(&'static str),
}

/// Enable using the `?` operator on io::Result
impl From<::std::io::Error> for Error {
    fn from(io_err: ::std::io::Error) -> Self {
        Error::IoError(io_err)
    }
}

/// Enable using the `?` operator on compress::Result
impl From<::compress::Error> for Error {
    fn from(compress_err: ::compress::Error) -> Self {
        Error::CompressionError(compress_err)
    }
}








fn identify_exr<R: Read>(read: &mut R) -> Result<bool> {
    let mut magic_num = [0; 4];
    read.read_exact(&mut magic_num)?;
    Ok(magic_num == self::MAGIC_NUMBER)
}

fn skip_identification_bytes<R: Read>(read: &mut R) -> Result<()> {
    if identify_exr(read)? {
        Ok(())

    } else {
        Err(Error::NotEXR)
    }
}

fn version<R: ReadBytesExt>(read: &mut R) -> Result<Version> {
    let version_and_flags = read.read_i32::<LittleEndian>()?;

    // take the 8 least significant bits, they contain the file format version number
    let version = (version_and_flags & 0x000F) as u8;

    // the 24 most significant bits are treated as a set of boolean flags
    let is_single_tile = version_and_flags.get_bit(9);
    let has_long_names = version_and_flags.get_bit(10);
    let has_deep_data = version_and_flags.get_bit(11);
    let has_multiple_parts = version_and_flags.get_bit(12);
    // all remaining bits except 9, 10, 11 and 12 are reserved and should be 0

    Ok(Version {
        file_format_version: version,
        is_single_tile, has_long_names,
        has_deep_data, has_multiple_parts,
    })
}

/// `peek` the next byte, and consume it if it is 0
fn skip_null_byte_if_present<R: Read + Seek>(read: &mut SeekBufRead<R>) -> Result<bool> {
    if read_u8(read)? == 0 {
        Ok(true)

    } else {
        // go back that wasted byte because its not 0
        // TODO benchmark peeking the buffer performance
        read.seek(SeekFrom::Current(-1))?;
        Ok(false)
    }
}


fn read_u8<R: ReadBytesExt>(read: &mut R) -> Result<u8> {
    read.read_u8().map_err(Error::from)
}

fn read_i32<R: ReadBytesExt>(read: &mut R) -> Result<i32> {
    read.read_i32::<LittleEndian>().map_err(Error::from)
}

fn read_f32<R: ReadBytesExt>(read: &mut R) -> Result<f32> {
    read.read_f32::<LittleEndian>().map_err(Error::from)
}

fn read_u32<R: ReadBytesExt>(read: &mut R) -> Result<u32> {
    read.read_u32::<LittleEndian>().map_err(Error::from)
}

fn read_u64<R: ReadBytesExt>(read: &mut R) -> Result<u64> {
    read.read_u64::<LittleEndian>().map_err(Error::from)
}

fn read_f64<R: ReadBytesExt>(read: &mut R) -> Result<f64> {
    read.read_f64::<LittleEndian>().map_err(Error::from)
}

fn null_terminated_text<R: ReadBytesExt>(read: &mut R) -> Result<Text> {
    let mut bytes = SmallVec::new();

    loop {
        match read_u8(read)? {
            0 => break,
            non_terminator => bytes.push(non_terminator),
        }
    }

    Ok(Text { bytes })
}

fn i32_sized_text<R: Read + Seek>(read: &mut SeekBufRead<R>, expected_attribute_bytes: Option<u32>) -> Result<Text> {
    let string_byte_length = expected_attribute_bytes
        .map(|u| Ok(u as i32)) // use expected attribute bytes if known,
        .unwrap_or_else(|| read_i32(read))?; // or read from bytes otherwise

    // batch-read small strings,
    // but carefully handle suspiciously large strings
    let bytes = if string_byte_length < 512 {
        // possibly problematic: should be char but this code handles it like unsigned char (u8)
        let mut bytes = vec![0; string_byte_length as usize];
        read.read_exact(&mut bytes)?;
        bytes

    } else {
        // TODO add tests

        // we probably have a ill-formed size because it is very large,
        // so avoid allocating too much memory at once
        let mut bytes = vec![0; 512];
        read.read_exact(&mut bytes)?;

        // read the remaining bytes one by one
        for _ in 0..(string_byte_length - 512) {
            bytes.push(read_u8(read)?);
        }

        bytes
    };

    Ok(Text { bytes: SmallVec::from_vec(bytes) })
}

fn box2i<R: ReadBytesExt>(read: &mut R) -> Result<I32Box2> {
    Ok(I32Box2 {
        x_min: read_i32(read)?, y_min: read_i32(read)?,
        x_max: read_i32(read)?, y_max: read_i32(read)?,
    })
}

fn box2f<R: ReadBytesExt>(read: &mut R) -> Result<F32Box2> {
    Ok(F32Box2 {
        x_min: read_f32(read)?, y_min: read_f32(read)?,
        x_max: read_f32(read)?, y_max: read_f32(read)?,
    })
}

fn channel<R: Read + Seek>(read: &mut SeekBufRead<R>) -> Result<Channel> {
    let name = null_terminated_text(read)?;

    let pixel_type = match read_i32(read)? {
        0 => PixelType::U32,
        1 => PixelType::F16,
        2 => PixelType::F32,
        _ => return Err(Error::Invalid("pixel_type"))
    };

    let is_linear = match read_u8(read)? {
        1 => true,
        0 => false,
        _ => return Err(Error::Invalid("pLinear"))
    };

    let reserved = [
        read.read_i8()?,
        read.read_i8()?,
        read.read_i8()?,
    ];

    let x_sampling = read_i32(read)?;
    let y_sampling = read_i32(read)?;

    Ok(Channel {
        name, pixel_type, is_linear,
        reserved, x_sampling, y_sampling,
    })
}

fn channel_list<R: Read + Seek>(read: &mut SeekBufRead<R>) -> Result<ChannelList> {
    let mut channels = SmallVec::new();
    while !skip_null_byte_if_present(read)? {
        channels.push(channel(read)?);
    }

    Ok(channels)
}

fn chromaticities<R: ReadBytesExt>(read: &mut R) -> Result<Chromaticities> {
    Ok(Chromaticities {
        red_x:   read_f32(read)?,   red_y:   read_f32(read)?,
        green_x: read_f32(read)?,   green_y: read_f32(read)?,
        blue_x:  read_f32(read)?,   blue_y:  read_f32(read)?,
        white_x: read_f32(read)?,   white_y: read_f32(read)?,
    })
}

fn compression<R: ReadBytesExt>(read: &mut R) -> Result<Compression> {
    use ::attributes::Compression::*;
    Ok(match read_u8(read)? {
        0 => None,
        1 => RLE,
        2 => ZIPSingle,
        3 => ZIP,
        4 => PIZ,
        5 => PXR24,
        6 => B44,
        7 => B44A,
        _ => return Err(Error::Invalid("compression")),
    })
}

fn environment_map<R: ReadBytesExt>(read: &mut R) -> Result<EnvironmentMap> {
    Ok(match read_u8(read)? {
        0 => EnvironmentMap::LatitudeLongitude,
        1 => EnvironmentMap::Cube,
        _ => return Err(Error::Invalid("environment map"))
    })
}

fn key_code<R: ReadBytesExt>(read: &mut R) -> Result<KeyCode> {
    Ok(KeyCode {
        film_manufacturer_code: read_i32(read)?,
        film_type: read_i32(read)?,
        film_roll_prefix: read_i32(read)?,
        count: read_i32(read)?,
        perforation_offset: read_i32(read)?,
        perforations_per_frame: read_i32(read)?,
        perforations_per_count: read_i32(read)?,
    })
}

fn line_order<R: ReadBytesExt>(read: &mut R) -> Result<LineOrder> {
    use ::attributes::LineOrder::*;
    Ok(match read_u8(read)? {
        0 => IncreasingY,
        1 => DecreasingY,
        2 => RandomY,
        _ => return Err(Error::Invalid("line order")),
    })
}

fn f32_array<R: ReadBytesExt>(read: &mut R, result: &mut [f32]) -> Result<()> {
    for i in 0..result.len() {
        result[i] = read_f32(read)?;
    }

    Ok(())
}

fn f32_matrix_3x3<R: ReadBytesExt>(read: &mut R) -> Result<[f32; 9]> {
    let mut result = [0.0; 9];
    f32_array(read, &mut result)?;
    Ok(result)
}

fn f32_matrix_4x4<R: ReadBytesExt>(read: &mut R) -> Result<[f32; 16]> {
    let mut result = [0.0; 16];
    f32_array(read, &mut result)?;
    Ok(result)
}

fn i32_sized_text_vector<R: Read + Seek>(read: &mut SeekBufRead<R>, attribute_value_byte_size: u32) -> Result<Vec<Text>> {
    let mut result = Vec::with_capacity(2);
    let mut processed_bytes = 0_usize;

    while processed_bytes < attribute_value_byte_size as usize {
        let text = i32_sized_text(read, None)?;
        processed_bytes += ::std::mem::size_of::<i32>(); // size i32 of the text
        processed_bytes += text.bytes.len();
        result.push(text);
    }

    debug_assert_eq!(processed_bytes, attribute_value_byte_size as usize);
    Ok(result)
}

fn preview<R: ReadBytesExt>(read: &mut R) -> Result<Preview> {
    let width = read_u32(read)?;
    let height = read_u32(read)?;
    let components_per_pixel = 4;

    // TODO should be seen as char, not unsigned char!
    let mut pixel_data = vec![0_u8; (width * height * components_per_pixel) as usize];
    read.read_exact(&mut pixel_data)?;

    Ok(Preview {
        width, height,
        pixel_data,
    })
}

fn tile_description<R: ReadBytesExt>(read: &mut R) -> Result<TileDescription> {
    let x_size = read_u32(read)?;
    let y_size = read_u32(read)?;

    // mode = level_mode + (rounding_mode * 16)
    let mode = read_u8(read)?;

    let level_mode = mode & 0b00001111; // FIXME that was just guessed
    let rounding_mode = mode >> 4; // FIXME that was just guessed

    println!("mode: {:?}, level: {:?}, rounding: {:?},", mode, level_mode, rounding_mode);

    let level_mode = match level_mode {
        0 => LevelMode::One,
        1 => LevelMode::MipMap,
        2 => LevelMode::RipMap,
        _ => return Err(Error::Invalid("level mode"))
    };

    let rounding_mode = match rounding_mode {
        0 => RoundingMode::Down,
        1 => RoundingMode::Up,
        _ => return Err(Error::Invalid("rounding mode"))
    };

    println!("mode: {:?}, level: {:?}, rounding: {:?},", mode, level_mode, rounding_mode);

    Ok(TileDescription { x_size, y_size, level_mode, rounding_mode, })
}


fn attribute_value<R: Read + Seek>(read: &mut SeekBufRead<R>, kind: &Text, byte_size: u32) -> Result<AttributeValue> {
    Ok(match kind.bytes.as_slice() {
        b"box2i" => AttributeValue::I32Box2(box2i(read)?),
        b"box2f" => AttributeValue::F32Box2(box2f(read)?),

        b"int"    => AttributeValue::I32(read_i32(read)?),
        b"float"  => AttributeValue::F32(read_f32(read)?),
        b"double" => AttributeValue::F64(read_f64(read)?),

        b"rational" => AttributeValue::Rational(read_i32(read)?, read_u32(read)?),
        b"timecode" => AttributeValue::TimeCode(read_u32(read)?, read_u32(read)?),

        b"v2i" => AttributeValue::I32Vec2(read_i32(read)?, read_i32(read)?),
        b"v2f" => AttributeValue::F32Vec2(read_f32(read)?, read_f32(read)?),
        b"v3i" => AttributeValue::I32Vec3(read_i32(read)?, read_i32(read)?, read_i32(read)?),
        b"v3f" => AttributeValue::F32Vec3(read_f32(read)?, read_f32(read)?, read_f32(read)?),

        b"chlist" => AttributeValue::ChannelList(channel_list(read)?),
        b"chromaticities" => AttributeValue::Chromaticities(chromaticities(read)?),
        b"compression" => AttributeValue::Compression(compression(read)?),
        b"envmap" => AttributeValue::EnvironmentMap(environment_map(read)?),

        b"keycode" => AttributeValue::KeyCode(key_code(read)?),
        b"lineOrder" => AttributeValue::LineOrder(line_order(read)?),

        b"m33f" => AttributeValue::F32Matrix3x3(f32_matrix_3x3(read)?),
        b"m44f" => AttributeValue::F32Matrix4x4(f32_matrix_4x4(read)?),

        b"preview" => AttributeValue::Preview(preview(read)?),
        b"string" => AttributeValue::Text(i32_sized_text(read, Some(byte_size))?),
        b"stringvector" => AttributeValue::TextVector(i32_sized_text_vector(read, byte_size)?),
        b"tiledesc" => AttributeValue::TileDescription(tile_description(read)?),

        _ => {
            println!("Unknown attribute type: {:?}", kind.to_string());
            return Err(Error::UnknownAttributeType { bytes_to_skip: byte_size as u32 })
        }
    })
}

// TODO parse lazily, skip size, ...
fn attribute<R: Read + Seek>(read: &mut SeekBufRead<R>) -> Result<Attribute> {
    let name = null_terminated_text(read)?;
    let kind = null_terminated_text(read)?;
    let size = read_i32(read)? as u32; // TODO .checked_cast.ok_or(err:negative)
    let value = attribute_value(read, &kind, size)?;
    Ok(Attribute { name, kind, value, })
}

fn header<R: Seek + Read>(read: &mut SeekBufRead<R>, file_version: Version) -> Result<Header> {
    let mut attributes = SmallVec::new();

    // these required attributes will be Some(usize) when encountered while parsing
    let mut tiles = None;
    let mut name = None;
    let mut kind = None;
    let mut version = None;
    let mut chunk_count = None;
    let mut max_samples_per_pixel = None;
    let mut channels = None;
    let mut compression = None;
    let mut data_window = None;
    let mut display_window = None;
    let mut line_order = None;
    let mut pixel_aspect = None;
    let mut screen_window_center = None;
    let mut screen_window_width = None;


    while !skip_null_byte_if_present(read)? {
        match attribute(read) {
            // skip unknown attribute values
            Err(Error::UnknownAttributeType { bytes_to_skip }) => {
                read.seek(SeekFrom::Current(bytes_to_skip as i64))?;
            },

            Err(other_error) => return Err(other_error),

            Ok(attribute) => {
                // save index when a required attribute is encountered
                let index = attributes.len();
                match attribute.name.bytes.as_slice() {
                    b"tiles" => tiles = Some(index),
                    b"name" => name = Some(index),
                    b"type" => kind = Some(index),
                    b"version" => version = Some(index),
                    b"chunkCount" => chunk_count = Some(index),
                    b"maxSamplesPerPixel" => max_samples_per_pixel = Some(index),
                    b"channels" => channels = Some(index),
                    b"compression" => compression = Some(index),
                    b"dataWindow" => data_window = Some(index),
                    b"displayWindow" => display_window = Some(index),
                    b"lineOrder" => line_order = Some(index),
                    b"pixelAspectRatio" => pixel_aspect = Some(index),
                    b"screenWindowCenter" => screen_window_center = Some(index),
                    b"screenWindowWidth" => screen_window_width = Some(index),
                    _ => {},
                }

                attributes.push(attribute)
            }
        }
    }

    let header = Header {
        attributes,
        indices: AttributeIndices {
            channels: channels.ok_or(Error::Missing("channels"))?,
            compression: compression.ok_or(Error::Missing("compression"))?,
            data_window: data_window.ok_or(Error::Missing("data window"))?,
            display_window: display_window.ok_or(Error::Missing("display window"))?,
            line_order: line_order.ok_or(Error::Missing("line order"))?,
            pixel_aspect: pixel_aspect.ok_or(Error::Missing("pixel aspect ratio"))?,
            screen_window_center: screen_window_center.ok_or(Error::Missing("screen window center"))?,
            screen_window_width: screen_window_width.ok_or(Error::Missing("screen window width"))?,

            tiles, name, kind,
            version, chunk_count,
            max_samples_per_pixel,
        },
    };

    if header.is_valid(file_version) {
        Ok(header)

    } else {
        Err(Error::Invalid("header"))
    }
}

fn headers<R: Seek + Read>(read: &mut SeekBufRead<R>, version: Version) -> Result<Headers> {
    Ok({
        if !version.has_multiple_parts {
            SmallVec::from_elem(header(read, version)?, 1)

        } else {
            let mut headers = SmallVec::new();
            while !skip_null_byte_if_present(read)? {
                headers.push(header(read, version)?);
            }

            headers
        }
    })
}

fn offset_table<R: Seek + Read>(
    read: &mut SeekBufRead<R>,
    version: Version, header: &Header
) -> Result<OffsetTable> {
    let entry_count = {
        if let Some(chunk_count_index) = header.indices.chunk_count {
            if let &AttributeValue::I32(chunk_count) = &header.attributes[chunk_count_index].value {
                chunk_count as usize

            } else {
                return Err(Error::Invalid("chunkCount type"))
            }
        } else {
            debug_assert!(
                !version.has_multiple_parts,
                "Multi-Part header does not have chunkCount, should have been checked"
            );

            // If not multipart and the chunkCount is not present,
            // the number of entries in the chunk table is computed
            // using the dataWindow and tileDesc attributes and the compression format
            let _data_window = header.attributes[header.indices.data_window]
                .value.to_i32_box_2().ok_or(Error::Invalid("dataWindow type"))?;

            let _tiles_index = header.indices.tiles
                .expect("tiles missing, should have been checked");

            let _tile_description = header.attributes[_tiles_index]
                .value.to_tile_description().ok_or(Error::Invalid("tileDesc type"))?;

            let _compression = header.attributes[header.indices.compression]
                .value.to_compression().ok_or(Error::Invalid("compression type"))?;

            return Err(Error::NotSupported(
                "computing chunk count by considering data_window, tiles, and compression"
            ))
        }
    };

    println!("offset table length is: {}", entry_count);
    let mut offsets = Vec::with_capacity(entry_count);

    for _ in 0..entry_count {
        offsets.push(read_u64(read)?);
    }

    Ok(offsets)
}

fn offset_tables<R: Seek + Read>(
    read: &mut SeekBufRead<R>,
    version: Version, headers: &Headers,
) -> Result<OffsetTables> {
    let mut tables = SmallVec::new();

    for i in 0..headers.len() {
        // one offset table for each header
        tables.push(offset_table(read, version, &headers[i])?);
    }

    Ok(tables)
}

fn scan_line_block<R: Seek + Read>(
    read: &mut SeekBufRead<R>, meta_data: &MetaData,
) -> Result<ScanLineBlock> {
    unimplemented!()
}

fn tile_block<R: Seek + Read>(
    read: &mut SeekBufRead<R>, meta_data: &MetaData,
) -> Result<TileBlock> {
    unimplemented!()
}

fn deep_scan_line_block<R: Seek + Read>(
    read: &mut SeekBufRead<R>,
    meta_data: &MetaData,
) -> Result<DeepScanLineBlock> {
    unimplemented!()
}

fn deep_tile_block<R: Seek + Read>(
    read: &mut SeekBufRead<R>,
    meta_data: &MetaData,
) -> Result<DeepTileBlock> {
    unimplemented!()
}

// TODO what about ordering? y-ordering? random? increasing? or only needed for processing?

fn multi_part_chunk<R: Seek + Read>(
    read: &mut SeekBufRead<R>,
    meta_data: &MetaData,
) -> Result<MultiPartChunk> {
    let part_number = read_u64(read)?;

    println!("chunk part number: {}, parts: {}", part_number, meta_data.headers.len());
    let header = &meta_data.headers.get(part_number as usize)
        .ok_or(Error::Invalid("chunk part number"))?;

    let kind_index = header.indices.kind.ok_or(Error::Missing("multiplart 'type' attribute"))?;
    let kind = &header.attributes[kind_index].value.to_text()
        .ok_or(Error::Invalid("multipart 'type' attribute-type"))?;

    Ok(MultiPartChunk {
        part_number,
        block: match kind.bytes.as_slice() {
            b"scanlineimage" => MultiPartBlock::ScanLine(scan_line_block(read, meta_data)?),
            b"tiledimage"    => MultiPartBlock::Tiled(tile_block(read, meta_data)?),
            b"deepscanline"  => MultiPartBlock::DeepScanLine(Box::new(deep_scan_line_block(read, meta_data)?)),
            b"deeptile"      => MultiPartBlock::DeepTile(Box::new(deep_tile_block(read, meta_data)?)),
            _ => return Err(Error::Invalid("multi-part block type"))
        },
    })
}


fn multi_part_chunks<R: Seek + Read>(
    read: &mut SeekBufRead<R>,
    meta_data: &MetaData,
) -> Result<Vec<MultiPartChunk>> {
    let mut chunks = Vec::new();
    for offset_table in &meta_data.offset_tables {
        chunks.reserve(offset_table.len());
        for _ in 0..offset_table.len() {
            chunks.push(multi_part_chunk(read, meta_data)?)
        }
    }

    Ok(chunks)
}

fn single_part_chunks<R: Seek + Read>(
    read: &mut SeekBufRead<R>,
    meta_data: &MetaData,
) -> Result<SinglePartChunks> {
    let header = meta_data.headers.get(0).expect("no header found");
    let offset_table = meta_data.offset_tables.get(0).expect("no offset table found");

    let kind_index = header.indices.kind
        .ok_or(Error::Missing("single-part 'type' attribute"))?;

    let kind = &header.attributes[kind_index].value.to_text()
        .ok_or(Error::Invalid("single-part 'type' attribute-type"))?;

    Ok(match kind.bytes.as_slice() {
        b"scanlineimage" => {
            let mut scan_line_blocks = Vec::with_capacity(offset_table.len());
            for _ in 0..offset_table.len() {
                scan_line_blocks.push(scan_line_block(read, meta_data)?)
            }

            SinglePartChunks::ScanLine(scan_line_blocks)
        },

        b"tiledimage" => {
            let mut tile_blocks = Vec::with_capacity(offset_table.len());
            for _ in 0..offset_table.len() {
                tile_blocks.push(tile_block(read, meta_data)?)
            }

            SinglePartChunks::Tile(tile_blocks)
        },

        // FIXME check if single-part needs to support deep data
        _ => return Err(Error::Invalid("single-part block type"))
    })
}

fn chunks<R: Seek + Read>(
    read: &mut SeekBufRead<R>,
    meta_data: &MetaData,
) -> Result<Chunks> {
    Ok({
        if meta_data.version.has_multiple_parts {
            Chunks::MultiPart(multi_part_chunks(read, meta_data)?)

        } else {
            Chunks::SinglePart(single_part_chunks(read, meta_data)?)
        }
    })
}

fn meta_data<R: Seek + Read>(read: &mut SeekBufRead<R>) -> Result<MetaData> {
    let version = version(read)?;
    println!("version: {:#?}", version);

    if !version.is_valid() {
        return Err(Error::Invalid("version value combination"))
    }

    let headers = headers(read, version)?;
    println!("headers: {:#?}", headers);

    let offset_tables = offset_tables(read, version, &headers)?;

    // TODO check if supporting version 2 implies supporting version 1
    Ok(MetaData { version, headers, offset_tables })
}



#[must_use]
pub fn read_file(path: &str) -> Result<RawImage> {
    read(::std::fs::File::open(path)?)
}

/// assumes that the provided reader is not buffered, and will create a buffer for it
#[must_use]
pub fn read<R: Read + Seek>(unbuffered: R) -> Result<RawImage> {
    read_seekable_buffer(&mut SeekBufRead::new(unbuffered))
}

#[must_use]
pub fn read_seekable_buffer<R: Read + Seek>(read: &mut SeekBufRead<R>) -> Result<RawImage> {
    skip_identification_bytes(read)?;
    let meta_data = meta_data(read)?;
    let chunks = chunks(read, &meta_data)?;
    println!("chunks: {:?}", chunks);

    Ok(::file::RawImage { meta_data, chunks, })
}

