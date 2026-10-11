//! Variable values: a device table, or a variation index resolved against GDEF's item variation
//! store, either at every master location (the oracle's form) or at one given location.
//!
//! Ported from fontc's `otl-normalizer` (MIT OR Apache-2.0).

use std::collections::BTreeSet;
use std::fmt;

use read_fonts::tables::layout::DeviceOrVariationIndex;
use read_fonts::tables::variations::{DeltaSetIndex, ItemVariationStore};
use read_fonts::types::F2Dot14;

use super::{Budget, read_err};
use crate::{Result, TablesError};

/// A device table's adjustments, or the value at each master location.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum DeviceOrDeltas {
    Device { start: u16, end: u16, values: Vec<i8> },
    Deltas(Vec<i32>),
}

/// How variation indices are resolved.
pub(super) enum Resolver<'a> {
    /// The font has no item variation store.
    None,
    /// Print the value at each master location.
    Masters(DeltaComputer<'a>),
    /// Fold the delta at these normalised coordinates into the value.
    At(ItemVariationStore<'a>, Vec<F2Dot14>),
}

/// A value plus an optional device table or set of master values.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct ResolvedValue {
    pub(super) default: i32,
    pub(super) device_or_deltas: Option<DeviceOrDeltas>,
}

impl ResolvedValue {
    pub(super) fn plain(default: i16) -> Self {
        ResolvedValue { default: i32::from(default), device_or_deltas: None }
    }

    pub(super) fn new(
        default: Option<i16>,
        device: Option<std::result::Result<DeviceOrVariationIndex<'_>, read_fonts::ReadError>>,
        resolver: &Resolver<'_>,
        budget: &mut Budget,
    ) -> Result<Self> {
        let default = default.unwrap_or_default();
        let Some(device) = device else { return Ok(ResolvedValue::plain(default)) };
        let device = device.map_err(read_err("a device or variation index table"))?;
        resolve(default, device, resolver, budget)
    }

    pub(super) fn is_zero(&self) -> bool {
        self.default == 0 && self.device_or_deltas.is_none()
    }

    /// Adds another value for the same pair, as the oracle does: master values add when both
    /// sides have them; a device table on either side is kept from `self` only.
    pub(super) fn add_in_place(&mut self, other: &ResolvedValue) {
        self.default = self.default.wrapping_add(other.default);
        if let (Some(DeviceOrDeltas::Deltas(d1)), Some(DeviceOrDeltas::Deltas(d2))) =
            (self.device_or_deltas.as_mut(), other.device_or_deltas.as_ref())
        {
            d1.iter_mut().zip(d2.iter()).for_each(|(a, b)| *a = a.wrapping_add(*b));
        }
    }
}

/// A value with its device table or variation index resolved.
pub(super) fn resolve(default: i16, device: DeviceOrVariationIndex<'_>, resolver: &Resolver<'_>, budget: &mut Budget) -> Result<ResolvedValue> {
    match device {
        DeviceOrVariationIndex::Device(device) => {
            let values: Vec<i8> = device.iter().collect();
            budget.charge(values.len().saturating_add(1))?;
            Ok(ResolvedValue {
                default: i32::from(default),
                device_or_deltas: Some(DeviceOrDeltas::Device { start: device.start_size(), end: device.end_size(), values }),
            })
        }
        DeviceOrVariationIndex::VariationIndex(index) => {
            let index: DeltaSetIndex = index.into();
            match resolver {
                Resolver::None => Err(TablesError::invalid("a value has a variation index but GDEF has no item variation store")),
                Resolver::Masters(computer) => {
                    let values = computer.master_values(i32::from(default), index, budget)?;
                    Ok(ResolvedValue { default: i32::from(default), device_or_deltas: Some(DeviceOrDeltas::Deltas(values)) })
                }
                Resolver::At(ivs, coords) => {
                    budget.charge(region_count(ivs, index).saturating_add(1))?;
                    let delta = ivs.compute_delta(index, coords).ok_or_else(|| TablesError::invalid("the item variation store cannot be read"))?;
                    Ok(ResolvedValue::plain_i32(i32::from(default).wrapping_add(delta.to_i32())))
                }
            }
        }
    }
}

impl ResolvedValue {
    fn plain_i32(default: i32) -> Self {
        ResolvedValue { default, device_or_deltas: None }
    }
}

/// The regions the subtable of `index` refers to, for charging the work of one delta.
fn region_count(ivs: &ItemVariationStore<'_>, index: DeltaSetIndex) -> usize {
    match ivs.item_variation_data().get(usize::from(index.outer)) {
        Some(Ok(data)) => usize::from(data.region_index_count()),
        _ => 0,
    }
}

/// Computes the value of a variation index at every master location: the peak of each region of
/// the store (or only of the regions `used_subtables` refer to), sorted.
pub(super) struct DeltaComputer<'a> {
    ivs: ItemVariationStore<'a>,
    locations: Vec<Vec<F2Dot14>>,
}

impl<'a> DeltaComputer<'a> {
    pub(super) fn new(ivs: ItemVariationStore<'a>, used_subtables: Option<&BTreeSet<u16>>, budget: &mut Budget) -> Result<Self> {
        let err = read_err("the item variation store");
        let region_list = ivs.variation_region_list().map_err(&err)?;
        let used_regions: Option<BTreeSet<usize>> = match used_subtables {
            None => None,
            Some(outers) => {
                let mut set = BTreeSet::new();
                let all = ivs.item_variation_data();
                for &outer in outers {
                    if let Some(data) = all.get(usize::from(outer)) {
                        let data = data.map_err(&err)?;
                        budget.charge(data.region_indexes().len())?;
                        set.extend(data.region_indexes().iter().map(|i| usize::from(i.get())));
                    }
                }
                Some(set)
            }
        };
        let mut locations = Vec::new();
        for (i, region) in region_list.variation_regions().iter().enumerate() {
            if used_regions.as_ref().is_some_and(|set| !set.contains(&i)) {
                continue;
            }
            let region = region.map_err(&err)?;
            let axes = region.region_axes();
            budget.charge(axes.len().saturating_add(1))?;
            locations.push(axes.iter().map(|axis| axis.peak_coord()).collect::<Vec<_>>());
        }
        locations.sort();
        Ok(DeltaComputer { ivs, locations })
    }

    pub(super) fn master_values(&self, coord: i32, index: DeltaSetIndex, budget: &mut Budget) -> Result<Vec<i32>> {
        let per_location = region_count(&self.ivs, index).saturating_add(1);
        let mut values = Vec::with_capacity(self.locations.len());
        for location in &self.locations {
            budget.charge(per_location)?;
            let delta = self.ivs.compute_delta(index, location).ok_or_else(|| TablesError::invalid("the item variation store cannot be read"))?;
            values.push(delta.to_i32().wrapping_add(coord));
        }
        Ok(values)
    }
}

impl fmt::Display for DeviceOrDeltas {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceOrDeltas::Device { start, end, values } => {
                write!(f, " [({start})")?;
                for (i, adj) in values.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{adj}")?;
                }
                write!(f, "({end})]")
            }
            DeviceOrDeltas::Deltas(deltas) => {
                f.write_str(" {")?;
                for (i, value) in deltas.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{value}")?;
                }
                f.write_str("}")
            }
        }
    }
}

impl fmt::Display for ResolvedValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.default)?;
        if let Some(d) = &self.device_or_deltas {
            write!(f, "{d}")?;
        }
        Ok(())
    }
}
