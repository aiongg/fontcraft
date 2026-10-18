//! The item variation store of a CFF2 Top DICT, read and written as fontTools' `VarStore`,
//! `VarRegionList` and `VarData` (`ttLib.tables.otData`, `otConverters.VarDataValue`; MIT).

use crate::Result;

use super::super::core::Rd;
use super::super::{XmlWriter, fixed_to_str};
use super::bad;

pub(super) struct VarStore {
    format: u16,
    regions: Option<RegionList>,
    data: Vec<Option<VarData>>,
}

struct RegionList {
    axis_count: u16,
    /// Per region, per axis: start, peak and end as F2DOT14.
    regions: Vec<Vec<[i16; 3]>>,
}

struct VarData {
    item_count: u16,
    num_shorts: u16,
    region_indexes: Vec<u16>,
    items: Vec<Vec<i32>>,
}

/// Reads the store the `VarStore` offset points to: a 16-bit length, then the store (to the
/// end of the table when the length is 65535).
pub(super) fn read(table: &[u8], at: usize) -> Result<VarStore> {
    let mut r = Rd::at(table, at);
    let len = usize::from(r.u16()?);
    let rest = r.rest();
    let data = if len == 65535 { rest } else { rest.get(..len).unwrap_or(rest) };
    let mut r = Rd::new(data);
    let format = r.u16()?;
    let region_offset = r.u32()? as usize;
    let count = usize::from(r.u16()?);
    let mut offsets = Vec::with_capacity(count);
    for _ in 0..count {
        offsets.push(r.u32()? as usize);
    }
    let regions = if region_offset == 0 { None } else { Some(read_regions(data, region_offset)?) };
    let mut var_data = Vec::with_capacity(count);
    for offset in offsets {
        var_data.push(if offset == 0 { None } else { Some(read_var_data(data, offset)?) });
    }
    Ok(VarStore { format, regions, data: var_data })
}

fn read_regions(data: &[u8], at: usize) -> Result<RegionList> {
    let mut r = Rd::at(data, at);
    let axis_count = r.u16()?;
    let region_count = r.u16()?;
    let need = usize::from(axis_count) * usize::from(region_count) * 6;
    if r.rest().len() < need {
        return Err(bad("the variation region list is cut short"));
    }
    let mut regions = Vec::with_capacity(usize::from(region_count));
    for _ in 0..region_count {
        let mut axes = Vec::with_capacity(usize::from(axis_count));
        for _ in 0..axis_count {
            axes.push([r.i16()?, r.i16()?, r.i16()?]);
        }
        regions.push(axes);
    }
    Ok(RegionList { axis_count, regions })
}

fn read_var_data(data: &[u8], at: usize) -> Result<VarData> {
    let mut r = Rd::at(data, at);
    let item_count = r.u16()?;
    let num_shorts = r.u16()?;
    let region_count = r.u16()?;
    let mut region_indexes = Vec::with_capacity(usize::from(region_count));
    for _ in 0..region_count {
        region_indexes.push(r.u16()?);
    }
    let long = num_shorts & 0x8000 != 0;
    let words = usize::from(num_shorts & 0x7FFF);
    let regions = usize::from(region_count);
    let (big, small) = (words.min(regions), words.max(regions));
    let row = if long { big * 4 + (small - big) * 2 } else { big * 2 + (small - big) };
    if r.rest().len() < row * usize::from(item_count) {
        return Err(bad("a VarData is cut short"));
    }
    let mut items = Vec::with_capacity(usize::from(item_count));
    for _ in 0..item_count {
        let mut values = Vec::with_capacity(small);
        for _ in 0..big {
            values.push(if long { r.i32()? } else { i32::from(r.i16()?) });
        }
        for _ in big..small {
            values.push(if long { i32::from(r.i16()?) } else { i32::from(r.i8()?) });
        }
        values.truncate(regions);
        items.push(values);
    }
    Ok(VarData { item_count, num_shorts, region_indexes, items })
}

impl VarStore {
    /// The region count of VarData `vsindex` (a negative index counts from the end, as a Python
    /// list index does).
    pub(super) fn regions(&self, vsindex: i64) -> Result<usize> {
        let len = self.data.len() as i64;
        let at = if vsindex < 0 { vsindex + len } else { vsindex };
        let var_data = usize::try_from(at).ok().and_then(|i| self.data.get(i)).and_then(|d| d.as_ref());
        var_data.map(|d| d.region_indexes.len()).ok_or_else(|| bad("vsindex names a VarData the store does not have"))
    }

    pub(super) fn write(&self, w: &mut XmlWriter) {
        let format = self.format.to_string();
        w.open("VarStore", &[("Format", &format)]);
        w.simple("Format", &[("value", &format)]);
        if let Some(list) = &self.regions {
            w.open("VarRegionList", &[]);
            w.comment(&format!("RegionAxisCount={}", list.axis_count));
            w.comment(&format!("RegionCount={}", list.regions.len()));
            for (i, region) in list.regions.iter().enumerate() {
                w.open("Region", &[("index", &i.to_string())]);
                for (j, axis) in region.iter().enumerate() {
                    w.open("VarRegionAxis", &[("index", &j.to_string())]);
                    for (name, v) in ["StartCoord", "PeakCoord", "EndCoord"].iter().zip(axis) {
                        w.simple(name, &[("value", &fixed_to_str(f64::from(*v) / 16384.0, 14))]);
                    }
                    w.close("VarRegionAxis");
                }
                w.close("Region");
            }
            w.close("VarRegionList");
        }
        w.comment(&format!("VarDataCount={}", self.data.len()));
        for (i, var_data) in self.data.iter().enumerate() {
            let index = i.to_string();
            let Some(d) = var_data else {
                w.simple("VarData", &[("index", &index), ("empty", "1")]);
                continue;
            };
            w.open("VarData", &[("index", &index)]);
            w.comment(&format!("ItemCount={}", d.item_count));
            w.simple("NumShorts", &[("value", &d.num_shorts.to_string())]);
            w.comment(&format!("VarRegionCount={}", d.region_indexes.len()));
            for (j, r) in d.region_indexes.iter().enumerate() {
                w.simple("VarRegionIndex", &[("index", &j.to_string()), ("value", &r.to_string())]);
            }
            for (j, item) in d.items.iter().enumerate() {
                let values = item.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", ");
                w.simple("Item", &[("index", &j.to_string()), ("value", &format!("[{values}]"))]);
            }
            w.close("VarData");
        }
        w.close("VarStore");
    }
}
