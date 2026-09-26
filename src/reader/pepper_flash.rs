//! The heap of Pepper Flash, as the Hammerfest strategy asks for it.
//!
//! Each answer is one of the layout's own reads, unchanged: the read cost
//! baseline holds that the seam costs nothing.

use hammerfest_core::atom;

use crate::avm1::{read_u64, Layout, Memory};
use crate::heap::{Avm1Heap, Object, ObjectReference, Slot, StringReference, Value};

/// A Pepper Flash heap, read with one layout.
#[derive(Copy, Clone, Debug)]
pub struct PepperFlashHeap {
    layout: Layout,
}

impl PepperFlashHeap {
    pub fn new(layout: Layout) -> Self {
        Self { layout }
    }

    pub fn layout(&self) -> Layout {
        self.layout
    }

    /// An atom, as a value. Only a float costs a read: it does not fit in its
    /// atom.
    fn decode(&self, memory: &dyn Memory, value: u64) -> Option<Value> {
        let decoded = match value {
            atom::NULL => Value::Null,
            atom::TRUE => Value::Bool(true),
            atom::FALSE => Value::Bool(false),
            _ => match atom::tag(value) {
                atom::TAG_INT => Value::Number(atom::as_int(value)? as f64),
                atom::TAG_DOUBLE => Value::Number(atom::decode_double(read_u64(
                    memory,
                    atom::double_at(value)?,
                )?)),
                atom::TAG_STRING => Value::String(StringReference(atom::ptr(value))),
                atom::TAG_OBJECT => Value::Object(ObjectReference(atom::ptr(value))),
                _ => Value::Other,
            },
        };
        Some(decoded)
    }
}

impl Avm1Heap for PepperFlashHeap {
    fn property(
        &self,
        memory: &dyn Memory,
        object: Object,
        key: &str,
        slot: &mut Slot,
    ) -> Option<Value> {
        let value = self.layout.get_cached(memory, object.0, key, &mut slot.0)?;
        self.decode(memory, value)
    }

    fn object(&self, memory: &dyn Memory, reference: ObjectReference) -> Option<Object> {
        self.layout.table_of(memory, reference.0).map(Object)
    }

    fn object_owning(
        &mut self,
        memory: &dyn Memory,
        reference: ObjectReference,
        key: &str,
    ) -> Option<Object> {
        self.layout
            .derive_so_tbl(memory, reference.0, key)
            .map(Object)
    }

    fn string_is(&self, memory: &dyn Memory, string: StringReference, text: &str) -> bool {
        self.layout.string_eq(memory, string.0, text)
    }

    fn is_object(&self, memory: &dyn Memory, object: Object) -> Option<bool> {
        read_u64(memory, object.0).map(|vtable| vtable == self.layout.tbl_vt)
    }
}
