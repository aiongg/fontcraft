//! An item variation store evaluated at one location: the delta of every (outer, inner) index,
//! as fontTools' `instantiateItemVariationStore` sums them for a full instance.

use read_fonts::tables::variations::ItemVariationStore;
use read_fonts::{FontData, FontRead};

use super::tuple_scalar;
use crate::{Result, TablesError};

/// Most delta-set rows read from one store, over all its subtables (each subtable holds at most
/// 65,535 rows; a store may hold 65,535 subtables, far more than any font uses).
const MAX_ROWS: usize = 4 << 20;
/// Most deltas read from one store (rows times regions).
const MAX_DELTAS: usize = 64 << 20;

/// The delta of each item at the location, by outer then inner index.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StoreDeltas {
    rows: Vec<Vec<f64>>,
}

/// The index that means "no variation".
pub const NO_VARIATION_INDEX: (u16, u16) = (0xFFFF, 0xFFFF);

impl StoreDeltas {
    /// Evaluates the store at `bytes[offset..]` (the store's own data) at `coords`, the
    /// normalised location with one value per `fvar` axis.
    pub fn read(bytes: &[u8], offset: usize, coords: &[f64]) -> Result<Self> {
        let data = bytes.get(offset..).ok_or_else(|| TablesError::invalid("an item variation store offset points past the end of its table"))?;
        let store = ItemVariationStore::read(FontData::new(data))
            .map_err(|e| TablesError::invalid(format!("an item variation store cannot be read: {e}")))?;
        Self::from_store(&store, coords)
    }

    /// Evaluates a store already read.
    pub fn from_store(store: &ItemVariationStore<'_>, coords: &[f64]) -> Result<Self> {
        let regions = store.variation_region_list().map_err(|e| TablesError::invalid(format!("the variation region list cannot be read: {e}")))?;
        let scalars: Vec<f64> = regions
            .variation_regions()
            .iter()
            .map(|r| {
                r.map(|r| {
                    tuple_scalar(
                        coords,
                        r.region_axes().iter().map(|a| (f2(a.start_coord().to_bits()), f2(a.peak_coord().to_bits()), f2(a.end_coord().to_bits()))),
                    )
                })
                .unwrap_or(0.0)
            })
            .collect();
        let mut rows = Vec::new();
        let (mut total_rows, mut total_deltas) = (0usize, 0usize);
        for i in 0..store.item_variation_data_count() {
            let Some(sub) = store.item_variation_data().get(usize::from(i)) else {
                rows.push(Vec::new());
                continue;
            };
            let Ok(sub) = sub else {
                // A null subtable has no items.
                rows.push(Vec::new());
                continue;
            };
            let region_indexes: Vec<usize> = sub.region_indexes().iter().map(|r| usize::from(r.get())).collect();
            let count = usize::from(sub.item_count());
            total_rows = total_rows.saturating_add(count);
            total_deltas = total_deltas.saturating_add(count.saturating_mul(region_indexes.len()));
            if total_rows > MAX_ROWS || total_deltas > MAX_DELTAS {
                return Err(TablesError::TooLarge { what: "the deltas of an item variation store", limit: MAX_DELTAS });
            }
            let mut out = Vec::with_capacity(count);
            for inner in 0..count {
                let Ok(inner) = u16::try_from(inner) else { break };
                let mut sum = 0.0;
                for (delta, region) in sub.delta_set(inner).zip(&region_indexes) {
                    sum += f64::from(delta) * scalars.get(*region).copied().unwrap_or(0.0);
                }
                out.push(sum);
            }
            rows.push(out);
        }
        Ok(Self { rows })
    }

    /// Deltas already summed, by outer then inner index.
    pub(super) fn from_rows(rows: Vec<Vec<f64>>) -> Self {
        Self { rows }
    }

    /// The delta of one item; 0 for [`NO_VARIATION_INDEX`] and for indexes the store lacks.
    pub fn get(&self, outer: u16, inner: u16) -> f64 {
        self.rows.get(usize::from(outer)).and_then(|r| r.get(usize::from(inner))).copied().unwrap_or(0.0)
    }

    /// The delta of a 32-bit variation index (outer in the high half).
    pub fn get_index(&self, index: u32) -> f64 {
        self.get((index >> 16) as u16, (index & 0xFFFF) as u16)
    }
}

fn f2(bits: i16) -> f64 {
    f64::from(bits) / 16384.0
}
