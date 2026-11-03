-- The adapter writes the key with resource content, in the same transaction as
-- creator edges. Person renames refresh affected keys in that transaction too.
-- A VALUE query or per-edge event amplifies every write into nested query plans.
DEFINE FIELD IF NOT EXISTS primary_creator_last_name ON resource TYPE option<string>;
DEFINE INDEX IF NOT EXISTS resource_primary_creator_last_name ON resource FIELDS primary_creator_last_name;
