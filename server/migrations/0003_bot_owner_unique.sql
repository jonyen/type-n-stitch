-- One bot per owner: makes a racing `ensure_bot` INSERT collide on
-- `owner_id` (not just on the now-random email), so the existing
-- unique-violation recovery in `ensure_bot` reliably re-reads the winner
-- instead of two concurrent first-time calls minting two bot rows.
CREATE UNIQUE INDEX users_owner_unique ON users(owner_id) WHERE owner_id IS NOT NULL;
