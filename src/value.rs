/// Tagged JS value. We keep it word-sized to mirror MQuickJS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Value(pub usize);

impl Value {
    const WORD_BYTES: usize = core::mem::size_of::<usize>();
    pub const TAG_INT: usize = 0;
    pub const TAG_PTR: usize = 1;
    pub const TAG_SPECIAL: usize = 3;

    pub const TAG_BOOL: usize = Self::TAG_SPECIAL | (0 << 2);
    pub const TAG_NULL: usize = Self::TAG_SPECIAL | (1 << 2);
    pub const TAG_UNDEFINED: usize = Self::TAG_SPECIAL | (2 << 2);
    pub const TAG_EXCEPTION: usize = Self::TAG_SPECIAL | (3 << 2);
    pub const TAG_SHORT_FUNC: usize = Self::TAG_SPECIAL | (4 << 2);
    pub const TAG_UNINITIALIZED: usize = Self::TAG_SPECIAL | (5 << 2);
    pub const TAG_STRING_CHAR: usize = Self::TAG_SPECIAL | (6 << 2);
    pub const TAG_CATCH_OFFSET: usize = Self::TAG_SPECIAL | (7 << 2);

    pub const TAG_SPECIAL_BITS: usize = 5;

    pub const NULL: Value = Value(Self::make_special(Self::TAG_NULL, 0));
    pub const UNDEFINED: Value = Value(Self::make_special(Self::TAG_UNDEFINED, 0));
    pub const UNINITIALIZED: Value = Value(Self::make_special(Self::TAG_UNINITIALIZED, 0));
    pub const FALSE: Value = Value(Self::make_special(Self::TAG_BOOL, 0));
    pub const TRUE: Value = Value(Self::make_special(Self::TAG_BOOL, 1));
    pub const EXCEPTION: Value = Value(Self::make_special(Self::TAG_EXCEPTION, 0));

    #[inline]
    pub const fn make_special(tag: usize, v: usize) -> usize {
        tag | (v << Self::TAG_SPECIAL_BITS)
    }

    #[inline]
    pub const fn from_int32(v: i32) -> Self {
        Value(((v as isize as usize) << 1) | Self::TAG_INT)
    }

    #[inline]
    pub fn int32(self) -> Option<i32> {
        if self.is_int() {
            Some((self.0 as isize >> 1) as i32)
        } else {
            None
        }
    }

    #[inline]
    pub fn is_int(self) -> bool {
        (self.0 & 1) == Self::TAG_INT
    }

    #[inline]
    pub fn is_ptr(self) -> bool {
        (self.0 & (Self::WORD_BYTES - 1)) == Self::TAG_PTR
    }

    #[inline]
    pub fn is_bool(self) -> bool {
        self.special_tag() == Self::TAG_BOOL
    }

    #[inline]
    pub fn is_null(self) -> bool {
        self == Self::NULL
    }

    #[inline]
    pub fn is_undefined(self) -> bool {
        self == Self::UNDEFINED
    }

    #[inline]
    pub fn is_uninitialized(self) -> bool {
        self == Self::UNINITIALIZED
    }

    #[inline]
    pub fn is_exception(self) -> bool {
        self == Self::EXCEPTION
    }

    #[inline]
    pub fn is_number(self) -> bool {
        self.is_int()
    }

    #[inline]
    pub fn special_value(self) -> usize {
        self.0 >> Self::TAG_SPECIAL_BITS
    }

    #[inline]
    pub fn special_tag(self) -> usize {
        self.0 & ((1 << Self::TAG_SPECIAL_BITS) - 1)
    }

    #[inline]
    pub fn new_bool(val: bool) -> Self {
        if val { Self::TRUE } else { Self::FALSE }
    }
}
