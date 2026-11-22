#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ByondVersion {
    pub major: u32,
    pub build: u32,
}

impl ByondVersion {
    pub const LATEST: ByondVersion = ByondVersion::new(516, 1689);

    pub const fn new(major: u32, build: u32) -> Self {
        ByondVersion { major, build }
    }

    pub const fn at_least(self, build: u32) -> bool {
        self.build >= build
    }

    pub fn read_as(self, ctx: &dreammaker::Context) {
        ctx.set_clamp_integer_literals(self.clamps_integer_literals());
    }
}

impl ByondVersion {
    pub(crate) const fn clamps_integer_literals(self) -> bool {
        !self.at_least(1644)
    }

    pub(crate) const fn has_proc_macro(self) -> bool {
        self.at_least(1644)
    }

    pub(crate) const fn is_516(self) -> bool {
        self.at_least(1648)
    }
}

impl ByondVersion {
    pub(crate) const fn folds_log_in_single(self) -> bool {
        self.at_least(1644)
    }

    pub(crate) const fn rounds_radians_to_single(self) -> bool {
        self.at_least(1644)
    }

    pub(crate) const fn rounds_colours_as_folded(self) -> bool {
        self.at_least(1647)
    }

    pub(crate) const fn clamps_hsv_colours(self) -> bool {
        self.at_least(1657)
    }

    pub(crate) const fn folds_nan_tests_cleanly(self) -> bool {
        self.build == 1670 || self.at_least(1672)
    }

    pub(crate) const fn folds_within_nan_tests(self) -> bool {
        self.at_least(1670)
    }
}

impl ByondVersion {
    pub(crate) const fn drops_pointer_index(self) -> bool {
        self.at_least(1668)
    }

    pub(crate) const fn derefs_made_pointer(self) -> bool {
        self.at_least(1668)
    }

    pub(crate) const fn keeps_safe_logical_null_tests(self) -> bool {
        self.at_least(1673)
    }

    pub(crate) const fn restores_cache_for_safe_element(self) -> bool {
        self.at_least(1685)
    }

    pub(crate) const fn keeps_zero_pick_weight(self) -> bool {
        self.at_least(1689)
    }
}

impl ByondVersion {
    pub(crate) const fn find_calls_compare_operator(self) -> bool {
        self.at_least(1674)
    }

    pub(crate) const fn discards_equivalence_result(self) -> bool {
        self.at_least(1674) && !self.at_least(1689)
    }

    pub(crate) const fn area_exit_calls_uncross(self) -> bool {
        self.at_least(1683)
    }
}

impl ByondVersion {
    pub(crate) const fn visits_mutable_appearance_at_root(self) -> bool {
        self.at_least(1643)
    }

    pub(crate) const fn defers_list_proc_initializers(self) -> bool {
        self.at_least(1648)
    }

    pub(crate) const fn runs_scoped_reads_last(self) -> bool {
        self.at_least(1685)
    }
}

impl Default for ByondVersion {
    fn default() -> Self {
        Self::LATEST
    }
}

impl std::fmt::Display for ByondVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.build)
    }
}
