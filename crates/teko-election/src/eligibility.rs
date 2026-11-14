//! Who may vote in which contest: the President is national, a governor is
//! state-wide, a mayor is municipal. Pure rules; the AUTHORITY enforces them
//! when issuing registration rights (the device check is only a courtesy:
//! a cloned or malicious device must not be able to widen its own rights).

use alloc::string::String;

use crate::error::ElectionError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum ScopeLevel {
    Unspecified = 0,
    /// Everyone (e.g. President).
    National = 1,
    /// Voters whose electoral domicile is in this state (e.g. Governor).
    State = 2,
    /// Voters whose electoral domicile is in this municipality (e.g. Mayor).
    Municipality = 3,
}

/// The jurisdiction a contest covers. `code`: empty for National; the 2-letter
/// UF for State (`"SP"`); the 7-digit IBGE code for Municipality (`"3550308"`).
#[derive(Clone, PartialEq, Eq, prost::Message)]
pub struct Scope {
    #[prost(enumeration = "ScopeLevel", tag = "1")]
    pub level: i32,
    #[prost(string, tag = "2")]
    pub code: String,
}

impl Scope {
    pub fn national() -> Self {
        Self {
            level: ScopeLevel::National as i32,
            code: String::new(),
        }
    }
    pub fn state(uf: &str) -> Self {
        Self {
            level: ScopeLevel::State as i32,
            code: String::from(uf),
        }
    }
    pub fn municipality(ibge: &str) -> Self {
        Self {
            level: ScopeLevel::Municipality as i32,
            code: String::from(ibge),
        }
    }

    pub fn validate(&self) -> Result<(), ElectionError> {
        let bad = ElectionError::InvalidJurisdiction;
        match self.level() {
            ScopeLevel::National if self.code.is_empty() => Ok(()),
            ScopeLevel::National => Err(bad("national scope takes no code")),
            ScopeLevel::State if is_uf(&self.code) => Ok(()),
            ScopeLevel::State => Err(bad("state code must be 2 upper-case letters")),
            ScopeLevel::Municipality if is_ibge(&self.code) => Ok(()),
            ScopeLevel::Municipality => Err(bad("municipality code must be 7 digits (IBGE)")),
            ScopeLevel::Unspecified => Err(bad("scope level missing")),
        }
    }
}

fn is_uf(s: &str) -> bool {
    s.len() == 2 && s.bytes().all(|b| b.is_ascii_uppercase())
}

fn is_ibge(s: &str) -> bool {
    s.len() == 7 && s.bytes().all(|b| b.is_ascii_digit())
}

/// A voter's electoral domicile (from the credential).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Domicile {
    pub state: String,
    pub municipality: String,
}

impl Domicile {
    pub fn new(state: &str, municipality: &str) -> Result<Self, ElectionError> {
        if !is_uf(state) {
            return Err(ElectionError::InvalidJurisdiction(
                "state must be 2 upper-case letters",
            ));
        }
        if !is_ibge(municipality) {
            return Err(ElectionError::InvalidJurisdiction(
                "municipality must be 7 digits",
            ));
        }
        Ok(Self {
            state: String::from(state),
            municipality: String::from(municipality),
        })
    }
}

/// May a voter with this domicile vote in a contest of this scope?
pub fn is_eligible(domicile: &Domicile, scope: &Scope) -> bool {
    match scope.level() {
        ScopeLevel::National => true,
        ScopeLevel::State => domicile.state == scope.code,
        ScopeLevel::Municipality => domicile.municipality == scope.code,
        ScopeLevel::Unspecified => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn president_is_national_governor_state_mayor_municipal() {
        let sp_capital = Domicile::new("SP", "3550308").unwrap();
        let rio = Domicile::new("RJ", "3304557").unwrap();
        assert!(is_eligible(&sp_capital, &Scope::national()));
        assert!(
            is_eligible(&rio, &Scope::national()),
            "everyone votes for President"
        );
        assert!(is_eligible(&sp_capital, &Scope::state("SP")));
        assert!(
            !is_eligible(&rio, &Scope::state("SP")),
            "a carioca does not vote for São Paulo's governor"
        );
        assert!(is_eligible(&sp_capital, &Scope::municipality("3550308")));
        assert!(!is_eligible(&rio, &Scope::municipality("3550308")));
        // same state, other municipality
        let campinas = Domicile::new("SP", "3509502").unwrap();
        assert!(is_eligible(&campinas, &Scope::state("SP")));
        assert!(!is_eligible(&campinas, &Scope::municipality("3550308")));
    }

    #[test]
    fn scopes_and_domiciles_are_validated() {
        assert!(Scope::national().validate().is_ok());
        assert!(Scope::state("SP").validate().is_ok());
        assert!(Scope::municipality("3550308").validate().is_ok());
        for bad in [
            Scope::state("sp"),
            Scope::state("SPX"),
            Scope::state(""),
            Scope::municipality("123"),
            Scope::municipality("35503O8"),
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
        let mut n = Scope::national();
        n.code = "SP".into();
        assert!(n.validate().is_err());
        assert!(
            Scope {
                level: 0,
                code: String::new()
            }
            .validate()
            .is_err()
        );
        assert!(Domicile::new("sp", "3550308").is_err());
        assert!(Domicile::new("SP", "355").is_err());
        assert!(!is_eligible(
            &Domicile::new("SP", "3550308").unwrap(),
            &Scope {
                level: 0,
                code: String::new()
            }
        ));
    }

    proptest! {
        #[test]
        fn eligibility_matches_the_jurisdiction_exactly(a in "[A-Z]{2}", b in "[A-Z]{2}", m1 in "[0-9]{7}", m2 in "[0-9]{7}") {
            let d = Domicile::new(&a, &m1).unwrap();
            prop_assert!(is_eligible(&d, &Scope::national()));
            prop_assert_eq!(is_eligible(&d, &Scope::state(&b)), a == b);
            prop_assert_eq!(is_eligible(&d, &Scope::municipality(&m2)), m1 == m2);
        }
    }
}
