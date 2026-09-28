//! Phase DAG -> critical-path schedule -> 4D visibility at day t.
use bim_core::*;
use std::collections::{HashMap, HashSet};

#[derive(Debug, thiserror::Error)]
pub enum ScheduleError {
    #[error("phase dependency cycle involving {0:?}")]
    Cycle(PhaseId),
    #[error("phase {0:?} depends on unknown phase {1:?}")]
    UnknownDependency(PhaseId, PhaseId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledPhase {
    pub id: PhaseId,
    pub start_day: u32,
    pub end_day: u32,
    pub critical: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Schedule {
    pub phases: Vec<ScheduledPhase>,
    pub total_days: u32,
}

impl Schedule {
    pub fn phase(&self, id: PhaseId) -> Option<&ScheduledPhase> {
        self.phases.iter().find(|p| p.id == id)
    }
    /// Phase has started by day t (its elements become visible).
    pub fn is_started(&self, id: PhaseId, t: u32) -> bool {
        self.phase(id).map_or(false, |p| t >= p.start_day)
    }
    pub fn is_complete(&self, id: PhaseId, t: u32) -> bool {
        self.phase(id).map_or(false, |p| t >= p.end_day)
    }
    /// Day an element appears: phase start + its offset, clamped to the phase end.
    pub fn element_start_day(&self, e: &Element) -> Option<u32> {
        self.phase(e.phase)
            .map(|p| (p.start_day + e.start_offset_days).min(p.end_day))
    }
    pub fn element_visible(&self, e: &Element, t: u32) -> bool {
        self.element_start_day(e).map_or(false, |d| t >= d)
    }
}

/// Kahn topological order over phase ids; error names one node on a cycle.
fn topo_order(phases: &[Phase]) -> Result<Vec<usize>, ScheduleError> {
    let idx: HashMap<PhaseId, usize> = phases.iter().enumerate().map(|(i, p)| (p.id, i)).collect();
    let mut indeg = vec![0usize; phases.len()];
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); phases.len()];
    for (i, p) in phases.iter().enumerate() {
        let mut seen = HashSet::new();
        for d in &p.depends_on {
            let &di = idx
                .get(d)
                .ok_or(ScheduleError::UnknownDependency(p.id, *d))?;
            if seen.insert(di) {
                indeg[i] += 1;
                succ[di].push(i);
            }
        }
    }
    let mut ready: Vec<usize> = (0..phases.len()).filter(|&i| indeg[i] == 0).collect();
    ready.reverse();
    let mut order = Vec::with_capacity(phases.len());
    while let Some(n) = ready.pop() {
        order.push(n);
        for &s in &succ[n] {
            indeg[s] -= 1;
            if indeg[s] == 0 {
                ready.push(s);
            }
        }
    }
    if order.len() != phases.len() {
        let stuck = (0..phases.len())
            .find(|&i| indeg[i] > 0)
            .map(|i| phases[i].id);
        return Err(ScheduleError::Cycle(stuck.unwrap_or(PhaseId(0))));
    }
    Ok(order)
}

/// Forward/backward-pass CPM. Zero-duration phases allowed.
pub fn schedule(phases: &[Phase]) -> Result<Schedule, ScheduleError> {
    let order = topo_order(phases)?;
    let idx: HashMap<PhaseId, usize> = phases.iter().enumerate().map(|(i, p)| (p.id, i)).collect();
    let dur = |i: usize| phases[i].duration_days;
    let mut es = vec![0u32; phases.len()];
    for &i in &order {
        es[i] = phases[i]
            .depends_on
            .iter()
            .map(|d| {
                let j = idx[d];
                es[j] + dur(j)
            })
            .max()
            .unwrap_or(0);
    }
    let total = (0..phases.len()).map(|i| es[i] + dur(i)).max().unwrap_or(0);
    let mut lf = vec![total; phases.len()];
    for &i in order.iter().rev() {
        let succ_min = phases
            .iter()
            .enumerate()
            .filter(|(_, p)| p.depends_on.contains(&phases[i].id))
            .map(|(j, _)| lf[j] - dur(j))
            .min();
        lf[i] = succ_min.unwrap_or(total);
    }
    let phases = order
        .iter()
        .map(|&i| ScheduledPhase {
            id: phases[i].id,
            start_day: es[i],
            end_day: es[i] + dur(i),
            critical: lf[i] - dur(i) == es[i],
        })
        .collect();
    Ok(Schedule {
        phases,
        total_days: total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ph(id: u32, d: u32, deps: &[u32]) -> Phase {
        Phase {
            id: PhaseId(id),
            name: id.to_string(),
            duration_days: d,
            depends_on: deps.iter().map(|&x| PhaseId(x)).collect(),
        }
    }
    #[test]
    fn linear_chain() {
        let s = schedule(&Project::sample().phases).unwrap();
        assert_eq!(s.total_days, 57);
        assert!(s.phases.iter().all(|p| p.critical));
        assert!(s.is_started(PhaseId(1), 10) && !s.is_started(PhaseId(1), 9));
    }
    #[test]
    fn parallel_branch_not_critical() {
        let s = schedule(&[
            ph(0, 5, &[]),
            ph(1, 10, &[0]),
            ph(2, 2, &[0]),
            ph(3, 1, &[1, 2]),
        ])
        .unwrap();
        assert_eq!(s.total_days, 16);
        assert!(!s.phase(PhaseId(2)).unwrap().critical);
    }
    #[test]
    fn cycle_detected() {
        assert!(matches!(
            schedule(&[ph(0, 1, &[1]), ph(1, 1, &[0])]),
            Err(ScheduleError::Cycle(_))
        ));
    }
    #[test]
    fn unknown_dep() {
        assert!(matches!(
            schedule(&[ph(0, 1, &[9])]),
            Err(ScheduleError::UnknownDependency(..))
        ));
    }
    #[test]
    fn element_offsets() {
        let p = Project::sample();
        let s = schedule(&p.phases).unwrap();
        let beam = p.elements.iter().find(|e| e.name == "Roof beam").unwrap();
        assert_eq!(s.element_start_day(beam), Some(18));
        assert!(!s.element_visible(beam, 17) && s.element_visible(beam, 18));
    }
    #[test]
    fn empty_ok() {
        assert_eq!(schedule(&[]).unwrap().total_days, 0);
    }
}
