
/// Timer instance (TON/TOF/TP)
#[derive(Debug, Clone)]
pub struct TimerInstance {
    pub kind: TimerKind,
    pub q: bool,         // Output
    pub et: i32,         // Elapsed time (ms)
    pub pt: i32,         // Preset time (ms)
    pub start_time: Option<std::time::Instant>,
    pub prev_in: bool,   // Previous input state (for edge detection)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimerKind {
    TON, // Timer On Delay
    TOF, // Timer Off Delay
    TP,  // Pulse Timer
}

impl Default for TimerInstance {
    fn default() -> Self {
        Self {
            kind: TimerKind::TON,
            q: false,
            et: 0,
            pt: 0,
            start_time: None,
            prev_in: false,
        }
    }
}

impl TimerInstance {
    pub fn new(kind: TimerKind) -> Self {
        Self {
            kind,
            ..Default::default()
        }
    }

    /// Execute one timer scan cycle
    pub fn execute(&mut self, in_value: bool, pt: i32) -> (bool, i32) {
        self.pt = pt;

        match self.kind {
            TimerKind::TON => self.execute_ton(in_value),
            TimerKind::TOF => self.execute_tof(in_value),
            TimerKind::TP => self.execute_tp(in_value),
        };

        self.prev_in = in_value;
        (self.q, self.et)
    }

    /// TON: Output Q goes TRUE after IN has been TRUE for PT duration
    fn execute_ton(&mut self, in_value: bool) {
        if in_value {
            if self.start_time.is_none() {
                self.start_time = Some(std::time::Instant::now());
            }
            self.et = self.start_time.unwrap().elapsed().as_millis() as i32;
            self.q = self.et >= self.pt;
        } else {
            self.start_time = None;
            self.et = 0;
            self.q = false;
        }
    }

    /// TOF: Output Q goes FALSE after IN has been FALSE for PT duration
    fn execute_tof(&mut self, in_value: bool) {
        if in_value {
            self.q = true;
            self.start_time = None;
            self.et = 0;
        } else if self.prev_in && !in_value {
            // Falling edge detected
            self.start_time = Some(std::time::Instant::now());
        }

        if !in_value && self.start_time.is_some() {
            self.et = self.start_time.unwrap().elapsed().as_millis() as i32;
            if self.et >= self.pt {
                self.q = false;
            }
        }
    }

    /// TP: Generates a pulse of PT duration when IN goes TRUE
    fn execute_tp(&mut self, in_value: bool) {
        if in_value && !self.prev_in {
            // Rising edge
            self.start_time = Some(std::time::Instant::now());
            self.q = true;
        }

        if self.q && self.start_time.is_some() {
            self.et = self.start_time.unwrap().elapsed().as_millis() as i32;
            if self.et >= self.pt {
                self.q = false;
            }
        }
    }
}
