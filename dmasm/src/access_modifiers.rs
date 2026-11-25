macro_rules! access_modifiers {
	( $( $name:ident = $value:tt ),* $(,)? ) => {
        $(
            #[allow(non_upper_case_globals)]
            pub const $name: u32 = $value;
        )*
	}
}

access_modifiers! {
    Usr = 0xFFCD,
    Src = 0xFFCE,
    Args = 0xFFCF,
    Dot = 0xFFD0,
    PtrDeref = 0xFFD5,
    Cache = 0xFFD8,
    Arg = 0xFFD9,
    Local = 0xFFDA,
    Global = 0xFFDB,
    SetCache = 0xFFDC,
    DynamicProc = 0xFFDD,
    DynamicVerb = 0xFFDE,
    StaticProc = 0xFFDF,
    StaticVerb = 0xFFE0,

    CacheKey = 0xFFE3,
    CacheIndex = 0xFFE4,

    World = 0xFFE5,
    Null = 0xFFE6,
    Initial = 0xFFE7,
    IsSaved = 0xFFE8,
    PtrRef = 0xFFEF,

    Caller = 0xFFF0,
    Callee = 0xFFF1,
}

pub fn is_access_modifier(value: u32) -> bool {
    (0xFFCD..=0xFFF1).contains(&value)
}
