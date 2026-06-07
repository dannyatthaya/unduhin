-- "Cancelled" was retired as a user-facing state: Pause stops a download and
-- Delete discards it, so a separate "cancel" added nothing. Convert any rows
-- left in the old `cancelled` state to `paused` so they stay resumable and
-- never sit hidden-but-counted in the UI. The `Status::Cancelled` enum variant
-- is kept on the Rust side purely for defensive parsing; nothing creates new
-- cancelled rows anymore.
UPDATE downloads SET status = 'paused' WHERE status = 'cancelled';
