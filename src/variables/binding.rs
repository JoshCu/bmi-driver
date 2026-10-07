//! Per-variable handles for the runner's timestep loop.
//!
//! A [`VarBinding`] resolves a model variable once — its type, C name, a reusable buffer and,
//! when the model exposes one, a `get_value_ptr` pointer into the model's own storage — so each
//! timestep is at most one BMI call (or a plain memory copy) with no hashing or allocation.
//!
//! BMI doesn't promise that `get_value_ptr` points at what `get_value`/`set_value` touch, or that
//! the pointer outlives an `update`, so a pointer starts in [`Access::Probe`]: values still go
//! through the BMI calls and are compared with the pointer's contents, and the pointer is
//! re-queried after each `update`. Once a non-zero value has matched (so a pure scale factor in
//! `get_value` can't hide behind zeros), or [`PROBE_STEPS`] steps in a row have matched for a
//! variable that stays zero, the binding switches to [`Access::Direct`]. Any mismatch drops the
//! pointer for good.

use std::ffi::{c_void, CString};
use std::ptr::NonNull;

use crate::adapters::to_cstring;
use crate::error::{function_failed, BmiResult};
use crate::traits::{Bmi, BmiValue, VarType};

/// Consecutive matching steps after which an all-zero variable's pointer is trusted.
const PROBE_STEPS: u32 = 48;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Access {
    /// `get_value` / `set_value` only.
    Call,
    /// Still calling BMI, but checking each value against the model's pointer.
    Probe {
        ptr: NonNull<c_void>,
        matches: u32,
        matched_nonzero: bool,
    },
    /// Reading/writing the model's memory directly.
    Direct(NonNull<c_void>),
}

pub(crate) struct VarBinding {
    name: String,
    cname: CString,
    buf: BmiValue,
    access: Access,
}

impl VarBinding {
    pub fn new(model: &mut dyn Bmi, name: &str, use_ptr: bool) -> BmiResult<Self> {
        let var_type = model.cached_type(name).unwrap_or_else(|| {
            let t = model.get_var_type(name).unwrap_or_default();
            let s = model.get_var_itemsize(name).unwrap_or(8);
            VarType::from_bmi(&t, s)
        });
        let itemsize = var_type
            .itemsize()
            .ok_or_else(|| function_failed(model.name(), format!("unknown type for '{name}'")))?;
        let nbytes = match model.cached_nbytes(name) {
            Some(n) => n,
            None => model.get_var_nbytes(name).unwrap_or(0),
        };
        let nbytes = usize::try_from(nbytes).unwrap_or(0);
        // Always room for one element, since the runner couples scalars.
        let count = (nbytes / itemsize).max(1);
        let buf = BmiValue::zeros(&var_type, count).expect("known var type");

        // Only alias model memory when the reported size is exactly our buffer's.
        let access = match use_ptr && nbytes == count * itemsize {
            true => match model.get_value_ptr(name) {
                Ok(Some(ptr)) => Access::Probe {
                    ptr,
                    matches: 0,
                    matched_nonzero: false,
                },
                _ => Access::Call,
            },
            false => Access::Call,
        };

        Ok(Self {
            name: name.to_string(),
            cname: to_cstring(name)?,
            buf,
            access,
        })
    }

    /// Set every element of the variable to `value`.
    pub fn set(&mut self, model: &mut dyn Bmi, value: f64) -> BmiResult<()> {
        self.buf.fill(value);
        if let Access::Direct(ptr) = self.access {
            let src = self.buf.as_bytes();
            // SAFETY: `ptr` came from the model's get_value_ptr for a variable of exactly
            // `src.len()` bytes, and was checked against set_value before being trusted.
            unsafe {
                std::ptr::copy_nonoverlapping(src.as_ptr(), ptr.as_ptr() as *mut u8, src.len())
            };
            return Ok(());
        }
        // SAFETY: `buf` holds max(nbytes, one element) bytes.
        let result = unsafe { model.set_value_from(&self.cname, &self.buf) };
        self.compare(result.is_ok());
        result
    }

    /// The variable's first element.
    pub fn get(&mut self, model: &dyn Bmi) -> BmiResult<f64> {
        if let Access::Direct(ptr) = self.access {
            let len = self.buf.as_bytes().len();
            // SAFETY: as in `set`; `buf` is exactly the variable's size.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    ptr.as_ptr() as *const u8,
                    self.buf.as_mut_ptr() as *mut u8,
                    len,
                )
            };
        } else {
            // SAFETY: `buf` holds max(nbytes, one element) bytes.
            let result = unsafe { model.get_value_into(&self.cname, &mut self.buf) };
            self.compare(result.is_ok());
            result?;
        }
        self.buf
            .scalar()
            .ok_or_else(|| function_failed(model.name(), "empty result"))
    }

    /// After `update`: drop a probed pointer if the model now reports a different one.
    pub fn recheck_ptr(&mut self, model: &mut dyn Bmi) {
        if let Access::Probe { ptr, .. } = self.access {
            if !matches!(model.get_value_ptr(&self.name), Ok(Some(p)) if p == ptr) {
                self.access = Access::Call;
            }
        }
    }

    /// End of timestep: trust a probed pointer once it has matched enough values.
    pub fn promote(&mut self) {
        if let Access::Probe {
            ptr,
            matches,
            matched_nonzero,
        } = self.access
        {
            if matched_nonzero || matches >= PROBE_STEPS {
                self.access = Access::Direct(ptr);
            }
        }
    }

    fn compare(&mut self, call_ok: bool) {
        let Access::Probe {
            ptr,
            ref mut matches,
            ref mut matched_nonzero,
        } = self.access
        else {
            return;
        };
        let expected = self.buf.as_bytes();
        // SAFETY: the model reported `ptr` for a variable of exactly `expected.len()` bytes.
        let actual =
            unsafe { std::slice::from_raw_parts(ptr.as_ptr() as *const u8, expected.len()) };
        if !call_ok || actual != expected {
            self.access = Access::Call;
            return;
        }
        *matches += 1;
        *matched_nonzero |= match &self.buf {
            BmiValue::Double(v) => v.iter().any(|&x| x != 0.0 && x.is_finite()),
            BmiValue::Float(v) => v.iter().any(|&x| x != 0.0 && x.is_finite()),
            BmiValue::Int(v) => v.iter().any(|&x| x != 0),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::BmiSloth;
    use std::collections::HashMap;

    fn sloth(value: &str) -> BmiSloth {
        let mut m = BmiSloth::new("SLOTH");
        let params = HashMap::from([("x(1,double,m,node)".to_string(), value.to_string())]);
        m.configure(&params).unwrap();
        m.initialize("/dev/null").unwrap();
        m
    }

    #[test]
    fn nonzero_match_goes_direct() {
        let mut m = sloth("2.5");
        let mut b = VarBinding::new(&mut m, "x", true).unwrap();
        assert!(matches!(b.access, Access::Probe { .. }));
        assert_eq!(b.get(&m).unwrap(), 2.5);
        b.recheck_ptr(&mut m);
        b.promote();
        assert!(matches!(b.access, Access::Direct(_)));

        b.set(&mut m, 7.0).unwrap();
        assert_eq!(m.get_value_f64("x").unwrap(), vec![7.0]);
        m.set_value_f64("x", &[3.0]).unwrap();
        assert_eq!(b.get(&m).unwrap(), 3.0);
    }

    #[test]
    fn zero_values_go_direct_after_probe_steps() {
        let mut m = sloth("0.0");
        let mut b = VarBinding::new(&mut m, "x", true).unwrap();
        for _ in 0..PROBE_STEPS - 1 {
            b.get(&m).unwrap();
            b.promote();
        }
        assert!(matches!(b.access, Access::Probe { .. }));
        b.get(&m).unwrap();
        b.promote();
        assert!(matches!(b.access, Access::Direct(_)));
    }

    #[test]
    fn moved_pointer_falls_back_to_calls() {
        let mut m = sloth("1.0");
        let mut b = VarBinding::new(&mut m, "x", true).unwrap();
        // Rebuilding the variables reallocates their storage.
        let params = HashMap::from([("x(1,double,m,node)".to_string(), "4.0".to_string())]);
        m.configure(&params).unwrap();
        b.recheck_ptr(&mut m);
        assert_eq!(b.access, Access::Call);
        assert_eq!(b.get(&m).unwrap(), 4.0);
    }

    #[test]
    fn opt_out_uses_calls() {
        let mut m = sloth("1.0");
        let mut b = VarBinding::new(&mut m, "x", false).unwrap();
        assert_eq!(b.access, Access::Call);
        b.set(&mut m, 9.0).unwrap();
        assert_eq!(b.get(&m).unwrap(), 9.0);
    }
}
