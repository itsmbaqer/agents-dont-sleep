use serde::Serialize;

/// Why the Mac is (or isn't) being held awake. `Holding` is the only "awake" outcome.
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum Reason {
    Unlicensed,
    Disabled,
    Paused,
    NoAgents,
    Thermal,
    LowPower,
    NotPluggedIn,
    Battery,
    Holding,
}

pub struct Inputs {
    pub licensed: bool,
    pub enabled: bool,
    pub paused: bool,
    pub working: usize,
    pub thermal: u8,
    pub thermal_limit: u8,
    pub low_power: bool,
    pub respect_low_power: bool,
    pub only_plugged_in: bool,
    pub on_ac: bool,
    /// None on desktops (no battery).
    pub battery: Option<u8>,
    pub cutoff: u8,
}

/// Safety checks come after "is there work at all" so the menu says "idle" rather than
/// "battery low" when nothing is running.
pub fn decide(i: &Inputs) -> Reason {
    use Reason::*;
    if !i.licensed {
        Unlicensed
    } else if !i.enabled {
        Disabled
    } else if i.paused {
        Paused
    } else if i.working == 0 {
        NoAgents
    } else if i.thermal >= i.thermal_limit {
        Thermal
    } else if i.low_power && i.respect_low_power {
        LowPower
    } else if i.only_plugged_in && !i.on_ac {
        NotPluggedIn
    } else if !i.on_ac && i.battery.is_some_and(|b| b <= i.cutoff) {
        Battery
    } else {
        Holding
    }
}

#[cfg(test)]
mod tests {
    use super::{Reason::*, *};

    fn base() -> Inputs {
        Inputs {
            licensed: true,
            enabled: true,
            paused: false,
            working: 1,
            thermal: 0,
            thermal_limit: 2,
            low_power: false,
            respect_low_power: true,
            only_plugged_in: false,
            on_ac: false,
            battery: Some(80),
            cutoff: 15,
        }
    }

    #[test]
    fn rules() {
        assert_eq!(decide(&base()), Holding);
        assert_eq!(decide(&Inputs { licensed: false, ..base() }), Unlicensed);
        assert_eq!(decide(&Inputs { enabled: false, ..base() }), Disabled);
        assert_eq!(decide(&Inputs { paused: true, ..base() }), Paused);
        assert_eq!(decide(&Inputs { working: 0, battery: Some(5), ..base() }), NoAgents);
        assert_eq!(decide(&Inputs { thermal: 2, ..base() }), Thermal);
        assert_eq!(decide(&Inputs { thermal: 2, thermal_limit: 3, ..base() }), Holding);
        assert_eq!(decide(&Inputs { low_power: true, ..base() }), LowPower);
        assert_eq!(decide(&Inputs { low_power: true, respect_low_power: false, ..base() }), Holding);
        assert_eq!(decide(&Inputs { only_plugged_in: true, ..base() }), NotPluggedIn);
        assert_eq!(decide(&Inputs { only_plugged_in: true, on_ac: true, ..base() }), Holding);
        assert_eq!(decide(&Inputs { battery: Some(15), ..base() }), Battery);
        assert_eq!(decide(&Inputs { battery: Some(15), on_ac: true, ..base() }), Holding);
        assert_eq!(decide(&Inputs { battery: None, ..base() }), Holding);
    }
}
