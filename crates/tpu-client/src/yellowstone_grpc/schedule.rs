//! A Yellowstone-specific UpcomingLeaderPredictor implementation
//!
//! This module provides an implementation of the UpcomingLeaderPredictor trait
//! tailored for Yellowstone, utilizing gRPC and RPC services to track the current slot
//! and predict upcoming leaders.
//!
//! # Strategy
//!
//! Predicts leaders from n-1 (previous), n (current), to n+X (future) to maximize
//! transaction landing probability by maintaining warm QUIC connections to all relevant leaders.
//!
//! # Safety
//!
//! This module is designed to be thread-safe and can be shared across multiple tasks.
//!
//! # Poisoning
//!
//! The slot tracker/managed schedule used in this implementation can be poisoned if the background task
//! updating it panics or is dropped.
//!
use {
    crate::{
        core::UpcomingLeaderPredictor, rpc::schedule::ManagedLeaderSchedule,
        slot::AtomicSlotTracker,
    },
    solana_pubkey::Pubkey,
    std::sync::Arc,
};

///
/// A Yellowstone-specific implementation of UpcomingLeaderPredictor
///
/// # Prediction Strategy
///
/// For n requested leaders, predicts from leader n-1 (previous) through leader n+(n-2) (future).
/// This ensures we always have connections to:
/// - Previous leader (might still accept transactions)
/// - Current leader (most likely to land)
/// - Future leaders (backup if current leader fails)
///
/// Example with n=5:
/// - Leader n-1 (previous)
/// - Leader n (current)
/// - Leader n+1, n+2, n+3 (next 3 leaders)
///
/// # Safety
///
/// This struct is cheaply-cloneable and can be shared between threads.
///
#[derive(Clone)]
pub struct YellowstoneUpcomingLeader {
    pub slot_tracker: Arc<AtomicSlotTracker>,
    pub managed_schedule: ManagedLeaderSchedule,
}

impl UpcomingLeaderPredictor for YellowstoneUpcomingLeader {
    fn try_predict_next_n_leaders(&self, n: usize) -> Vec<Pubkey> {
        if n == 0 {
            return Vec::new();
        }

        let slot = self.slot_tracker.load().expect("load");
        let reminder = slot % 4;

        // Calculate the current leader's slot boundary
        let current_leader_boundary = slot.saturating_sub(reminder);

        // Start from the PREVIOUS leader (n-1)
        // This ensures we have a connection even if the current leader is almost done
        let start_boundary = current_leader_boundary.saturating_sub(4);

        tracing::debug!(
            "[YellowstoneUpcomingLeader] Predicting {} leaders starting from slot {} (current_slot={}, reminder={}/4, current_boundary={}, previous_boundary={})",
            n,
            start_boundary,
            slot,
            reminder,
            current_leader_boundary,
            start_boundary
        );

        // Generate n leaders starting from previous leader
        // This gives us: n-1, n, n+1, ..., n+(n-2)
        let leaders: Vec<Pubkey> = (0..n)
            .map(|i| start_boundary + (i * 4) as u64)
            .filter_map(|leader_slot_boundary| {
                match self.managed_schedule.get_leader(leader_slot_boundary) {
                    Ok(Some(leader)) => {
                        tracing::trace!(
                            "[YellowstoneUpcomingLeader] Predicted leader at slot_boundary={}: {}",
                            leader_slot_boundary,
                            leader
                        );
                        Some(leader)
                    }
                    Ok(None) => {
                        tracing::debug!(
                            "[YellowstoneUpcomingLeader] No leader found for slot_boundary={}",
                            leader_slot_boundary
                        );
                        None
                    }
                    Err(e) => {
                        tracing::warn!(
                            "[YellowstoneUpcomingLeader] Failed to get leader for slot_boundary={}: {:?}",
                            leader_slot_boundary,
                            e
                        );
                        None
                    }
                }
            })
            .collect();

        tracing::debug!(
            "[YellowstoneUpcomingLeader] Successfully predicted {}/{} leaders",
            leaders.len(),
            n
        );

        leaders
    }
}
