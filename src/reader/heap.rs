//! The AVM1 object graph, as the Hammerfest strategy sees it.
//!
//! A Flash player keeps objects, their properties and their values in a
//! layout of its own. The strategy that finds and reads the game only needs to
//! ask two things: what a property holds, and which object a reference leads
//! to. Each player answers in its own way, behind [`Avm1Heap`].
//!
//! See `docs/specs/ruffle-support.md#design`.

use crate::avm1::Memory;
use crate::scan::Scan;

/// An object whose properties can be read.
///
/// What the address is -- a property table, a garbage collected cell -- is
/// the business of the heap that gave it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Object(pub u64);

/// What a property holds when it points at an object.
///
/// It is not an [`Object`] yet: following it costs reads, so the strategy
/// follows only the references it needs.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ObjectReference(pub u64);

/// What a property holds when it is a string. It is compared, never copied.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct StringReference(pub u64);

/// Where a property sat the last time it was read.
///
/// Finding a property from scratch walks its object. The slot makes the next
/// read one look, and it is checked before it is trusted.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Slot(pub u64);

/// A value, whatever player holds it.
///
/// There is no integer: Ruffle has none, and a Pepper Flash integer is exact
/// in an `f64` up to 2^53.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Value {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(StringReference),
    Object(ObjectReference),
    /// Anything the strategy never reads: a clip, a native object.
    Other,
}

impl Value {
    pub fn as_bool(self) -> Option<bool> {
        match self {
            Value::Bool(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_number(self) -> Option<f64> {
        match self {
            Value::Number(value) => Some(value),
            _ => None,
        }
    }

    /// The number, when it is a whole one.
    ///
    /// A level, a timer counted in frames: the game writes them as integers.
    /// A number with a fraction there is not the property we think it is.
    pub fn as_whole_number(self) -> Option<i64> {
        let number = self.as_number()?;
        // No `f64::fract` without `std`. A whole number survives the trip
        // through an integer; a fraction, an infinity or a NaN does not.
        let whole = number as i64;
        (whole as f64 == number).then_some(whole)
    }

    pub fn as_string(self) -> Option<StringReference> {
        match self {
            Value::String(string) => Some(string),
            _ => None,
        }
    }

    pub fn as_object(self) -> Option<ObjectReference> {
        match self {
            Value::Object(reference) => Some(reference),
            _ => None,
        }
    }
}

/// The questions the Hammerfest strategy asks of a Flash player's heap.
///
/// Every answer is checked before it is given. A doubt answers `None`, never
/// a default: see `reader::refuses-rather-than-defaults`.
pub trait Avm1Heap {
    /// What the property `key` of `object` holds, or `None` when the object
    /// has no such property, or it cannot be read.
    fn property(
        &self,
        memory: &dyn Memory,
        object: Object,
        key: &str,
        slot: &mut Slot,
    ) -> Option<Value>;

    /// The object a reference leads to.
    fn object(&self, memory: &dyn Memory, reference: ObjectReference) -> Option<Object>;

    /// The object a reference leads to, which must own `key`.
    ///
    /// The heap may learn from it how its objects are laid out, so the first
    /// object of a candidate goes through here.
    fn object_owning(
        &mut self,
        memory: &dyn Memory,
        reference: ObjectReference,
        key: &str,
    ) -> Option<Object>;

    /// Is this string exactly `text`?
    fn string_is(&self, memory: &dyn Memory, string: StringReference, text: &str) -> bool;

    /// Is this still an object of the heap? `None` when it cannot be read.
    ///
    /// Memory is recycled: an object kept from one read to the next must be
    /// checked again before it is trusted.
    fn is_object(&self, memory: &dyn Memory, object: Object) -> Option<bool>;

    /// The name of the layout this heap is read with, for the log.
    fn layout_name(&self) -> &'static str;
}

/// A Flash player, as the search sees it: where the objects that own a key
/// are, and what a search learns for the next one.
// Every future runs on the one thread of the WebAssembly sandbox, so no
// caller needs it to be `Send`.
#[allow(async_fn_in_trait)]
pub trait FlashPlayer {
    /// The heap an object found by this player is read with.
    type Heap: Avm1Heap + Copy;

    /// The first object that owns `key` and that `accept` keeps.
    ///
    /// `fresh` is the memory that changed since the last search, `all` every
    /// region, in the order to sweep them. The player may reorder `all`.
    async fn objects_owning<T>(
        &mut self,
        memory: &dyn Memory,
        key: &str,
        fresh: &[(u64, u64)],
        all: &mut [(u64, u64)],
        cost: &mut Scan<'_>,
        accept: impl FnMut(Self::Heap, Object) -> Option<T>,
    ) -> Option<T>;

    /// A game was read with this heap: the next search may start from it.
    fn learn(&mut self, heap: &Self::Heap);
}
