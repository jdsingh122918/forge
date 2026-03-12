// --- module: executor.rs ---
// Depends on: phase_manager, reporter

use crate::phase_manager::PhaseManager;
use crate::reporter::Reporter;

pub struct Executor {
    phase_manager: PhaseManager,
    reporter: Reporter,
}

impl Executor {
    pub fn new(phase_manager: PhaseManager, reporter: Reporter) -> Self {
        Self { phase_manager, reporter }
    }

    pub fn run(&mut self, spec: &str) -> Result<(), String> {
        let phases = self.phase_manager.plan(spec)?;
        for phase in &phases {
            self.reporter.log_phase_start(&phase.name);
            let result = self.execute_phase(phase)?;
            self.phase_manager.mark_complete(&phase.name, result)?;
            self.reporter.log_phase_end(&phase.name, result);
        }
        self.reporter.finalize();
        Ok(())
    }

    fn execute_phase(&self, phase: &Phase) -> Result<bool, String> {
        // Run the phase logic
        println!("Executing phase: {}", phase.name);
        Ok(true)
    }
}

pub struct Phase {
    pub name: String,
    pub budget: u32,
}

// --- module: phase_manager.rs ---
// Depends on: executor (for phase status), reporter (for logging)

use crate::executor::{Executor, Phase};
use crate::reporter::Reporter;

pub struct PhaseManager {
    phases: Vec<Phase>,
    reporter: Reporter,
    executor_ref: Option<*mut Executor>, // raw pointer to break ownership cycle
}

impl PhaseManager {
    pub fn new(reporter: Reporter) -> Self {
        Self {
            phases: Vec::new(),
            reporter,
            executor_ref: None,
        }
    }

    pub fn set_executor(&mut self, executor: &mut Executor) {
        self.executor_ref = Some(executor as *mut Executor);
    }

    pub fn plan(&mut self, spec: &str) -> Result<Vec<Phase>, String> {
        self.reporter.log_event("Planning phases...");
        let phases = vec![
            Phase { name: "setup".to_string(), budget: 5 },
            Phase { name: "implement".to_string(), budget: 10 },
            Phase { name: "test".to_string(), budget: 3 },
        ];
        self.phases = phases.clone();
        Ok(phases)
    }

    pub fn mark_complete(&mut self, phase_name: &str, success: bool) -> Result<(), String> {
        self.reporter.log_event(&format!("Phase {} complete: {}", phase_name, success));

        // Circular: PhaseManager reaches back into Executor to check state
        if let Some(executor_ptr) = self.executor_ref {
            let _executor = unsafe { &*executor_ptr };
            // Access executor state to decide re-planning
        }
        Ok(())
    }

    pub fn get_remaining_budget(&self) -> u32 {
        self.phases.iter().map(|p| p.budget).sum()
    }
}

// --- module: reporter.rs ---
// Depends on: executor (for run state), phase_manager (for phase list)

use crate::executor::Executor;
use crate::phase_manager::PhaseManager;

pub struct Reporter {
    log: Vec<String>,
    executor_ref: Option<*const Executor>,
    phase_manager_ref: Option<*const PhaseManager>,
}

impl Reporter {
    pub fn new() -> Self {
        Self {
            log: Vec::new(),
            executor_ref: None,
            phase_manager_ref: None,
        }
    }

    pub fn set_executor(&mut self, executor: &Executor) {
        self.executor_ref = Some(executor as *const Executor);
    }

    pub fn set_phase_manager(&mut self, pm: &PhaseManager) {
        self.phase_manager_ref = Some(pm as *const PhaseManager);
    }

    pub fn log_phase_start(&mut self, name: &str) {
        self.log.push(format!("[START] {}", name));
    }

    pub fn log_phase_end(&mut self, name: &str, success: bool) {
        self.log.push(format!("[END] {} success={}", name, success));
    }

    pub fn log_event(&mut self, msg: &str) {
        self.log.push(format!("[EVENT] {}", msg));
    }

    pub fn finalize(&self) {
        // Circular: Reporter reaches into PhaseManager to get remaining budget
        if let Some(pm_ptr) = self.phase_manager_ref {
            let pm = unsafe { &*pm_ptr };
            let remaining = pm.get_remaining_budget();
            println!("Remaining budget: {}", remaining);
        }

        // Circular: Reporter reaches into Executor for final state
        if let Some(exec_ptr) = self.executor_ref {
            let _exec = unsafe { &*exec_ptr };
            println!("Final report generated");
        }
    }

    pub fn get_log(&self) -> &[String] {
        &self.log
    }
}

// --- Wiring: shows the circular initialization problem ---

pub fn build_system(spec: &str) -> Result<(), String> {
    // Circular dependency forces unsafe multi-phase initialization:
    let mut reporter = Reporter::new();
    let mut phase_manager = PhaseManager::new(reporter.clone());
    let mut executor = Executor::new(phase_manager.clone(), reporter.clone());

    // Must set back-references after construction
    phase_manager.set_executor(&mut executor);
    reporter.set_executor(&executor);
    reporter.set_phase_manager(&phase_manager);

    executor.run(spec)
}
