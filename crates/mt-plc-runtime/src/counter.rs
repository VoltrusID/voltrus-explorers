/// Counter instance (CTU/CTD/CTUD)
#[derive(Debug, Clone)]
pub struct CounterInstance {
    pub kind: CounterKind,
    pub q: bool, // Output
    pub cv: i32, // Current value
    pub pv: i32, // Preset value
    pub prev_cu: bool,
    pub prev_cd: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CounterKind {
    CTU,  // Count Up
    CTD,  // Count Down
    CTUD, // Count Up/Down
}

impl Default for CounterInstance {
    fn default() -> Self {
        Self {
            kind: CounterKind::CTU,
            q: false,
            cv: 0,
            pv: 0,
            prev_cu: false,
            prev_cd: false,
        }
    }
}

impl CounterInstance {
    pub fn new(kind: CounterKind) -> Self {
        Self {
            kind,
            ..Default::default()
        }
    }

    /// CTU: Count up on rising edge of CU, reset with R
    pub fn execute_ctu(&mut self, cu: bool, r: bool, pv: i32) -> (bool, i32) {
        self.pv = pv;

        if r {
            self.cv = 0;
            self.q = false;
        } else {
            // Rising edge detection on CU
            if cu && !self.prev_cu {
                self.cv += 1;
            }
            self.q = self.cv >= self.pv;
        }

        self.prev_cu = cu;
        (self.q, self.cv)
    }

    /// CTD: Count down on rising edge of CD, load with LD
    pub fn execute_ctd(&mut self, cd: bool, ld: bool, pv: i32) -> (bool, i32) {
        self.pv = pv;

        if ld {
            self.cv = self.pv;
            self.q = false;
        } else {
            // Rising edge detection on CD
            if cd && !self.prev_cd {
                self.cv -= 1;
            }
            self.q = self.cv <= 0;
        }

        self.prev_cd = cd;
        (self.q, self.cv)
    }

    /// CTUD: Combined up/down counter
    pub fn execute_ctud(&mut self, cu: bool, cd: bool, r: bool, ld: bool, pv: i32) -> (bool, i32) {
        self.pv = pv;

        if r {
            self.cv = 0;
        } else if ld {
            self.cv = self.pv;
        } else {
            if cu && !self.prev_cu {
                self.cv += 1;
            }
            if cd && !self.prev_cd {
                self.cv -= 1;
            }
        }

        self.q = self.cv >= self.pv;
        self.prev_cu = cu;
        self.prev_cd = cd;
        (self.q, self.cv)
    }
}
