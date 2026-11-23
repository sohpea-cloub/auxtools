use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Id {
    ParseError,
    StringError,
    ExpectedLValue,
    ExpectedFieldReference,
    ExpectedEnd,
    UnexpectedRange,
    UnexpectedGlobal,
    UnexpectedArgList,
    UnexpectedProbability,
    UnexpectedNamedArguments,
    UnsupportedPrefabWithVars,
    UnsupportedBuiltin,
    UnsupportedImplicitNew,
    UnsupportedRelativeCall,
    UnsupportedImplicitLocate,
    UnsupportedStringInterpolation,
    UnsupportedInput,
    UnsupportedCompilerMacro,
    UnsupportedStatement,
    UndefinedVar,
    AmbiguousListConstructor,
    NotInByond,
    InvalidLocateArgs,
    BadProcReference,
    IncorrectArgCount,
    MissingArgument,
    TooManyArguments,
    CompilerPanicked,
    NoSuchProc,
    ImplicitNewUntyped,
    UnsupportedSetting,
    FoldedAngle,
    BitOperandOutOfRange,
    FoldedRgb,
    ProcPathNotDeclared,
    ByondShiftAssignDiscarded,
    ByondIncrementOperator,
    ByondPowerCallsMultiply,
    ByondCompareOperatorInFind,
    ByondEquivalenceDiscarded,
    ByondAreaUncross,
    ByondListLoopsAcrossTry,
    ByondContinueToDoWhile,
    ByondDerefOfReference,
    ByondPointerIndexDropped,
    ByondSafeChainReceiver,
    ByondSafeLogicalAssign,
    ByondSafeListFromCache,
    ByondFoldedTestFlag,
    ByondZeroWeightPick,
    ByondDefaultDivisionDropped,
    ParentChanged,
    ReparentedToRoot,
    MovedVarConflicts,
    InitializerFailed,
    ChangedInitializer,
    NewTypeWithoutParent,
    UnsupportedBuiltinDefault,
    BuiltinDefaultNotRestorable,
    ConstInitializerRequired,
    ConstValueFailed,
    ProcDeclarationChanged,
    DuplicateDefinition,
    ValueWithoutDeclaration,
    SettingChanged,
    FixedParseError,
    MapsChanged,
    BuildFlagsChanged,
    RemovedProcRetained,
    RemovedTypeRetained,
    RemovedVarRetained,
    RemovedGlobalRetained,
    UnreachableGlobalDefinition,
    NewVarComputedDefault,
    ComputedDefaultNotApplied,
    BuiltinDefaultNewInstancesOnly,
    ConstRefusedByDreamMaker,
    ConstUsesEditedResource,
    GlobalInitKept,

    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Span {
    pub file: Option<String>,
    pub line: u32,
    pub column: Option<u32>,
}

impl Span {
    pub fn line(line: u32) -> Self {
        Span {
            file: None,
            line,
            column: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Diagnostic {
    pub id: Id,
    pub severity: Severity,
    pub span: Option<Span>,
    pub subject: Option<String>,
    pub message: String,
}

impl Diagnostic {
    pub fn error(id: Id, message: impl Into<String>) -> Self {
        Diagnostic {
            id,
            severity: Severity::Error,
            span: None,
            subject: None,
            message: message.into(),
        }
    }

    pub fn warning(id: Id, message: impl Into<String>) -> Self {
        Diagnostic {
            severity: Severity::Warning,
            ..Diagnostic::error(id, message)
        }
    }

    pub fn at(mut self, span: Option<Span>) -> Self {
        self.span = span;
        self
    }

    pub fn about(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }

    pub fn line(&self) -> Option<u32> {
        self.span.as_ref().map(|span| span.line)
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(Span {
            file: Some(file),
            line,
            ..
        }) = &self.span
        {
            write!(f, "{file}:{line}: ")?;
        }
        if let Some(subject) = &self.subject {
            write!(f, "{subject}: ")?;
        }
        if let Some(Span {
            file: None, line, ..
        }) = &self.span
        {
            write!(f, "line {line}: ")?;
        }
        f.write_str(&self.message)
    }
}

impl std::error::Error for Diagnostic {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_diagnostic_displays_as_dmasm_writes_it() {
        let warning = Diagnostic::warning(Id::UndefinedVar, "x");
        assert_eq!(warning.to_string(), "x");
        let at = |file: Option<&str>| {
            Some(Span {
                file: file.map(str::to_owned),
                line: 3,
                column: Some(2),
            })
        };
        assert_eq!(warning.clone().at(at(None)).to_string(), "line 3: x");
        assert_eq!(
            warning.clone().at(at(None)).about("/proc/f").to_string(),
            "/proc/f: line 3: x"
        );
        assert_eq!(
            warning.at(at(Some("a.dm"))).about("/proc/f").to_string(),
            "a.dm:3: /proc/f: x"
        );
    }
}
