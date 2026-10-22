BEGIN TRANSACTION;
  DEFINE FIELD IF NOT EXISTS primary_creator_last_name ON resource TYPE option<string>
    VALUE {
      LET $resource = $this.id;
      LET $first = (
        SELECT
          order,
          credited_as,
          in.name AS name
        FROM created
        WHERE out = $resource
        ORDER BY order
        LIMIT 1
      )[0];
      RETURN IF $first.credited_as {
        array::last(string::split($first.credited_as, ' '));
      } ELSE IF $first.name.last {
        $first.name.last;
      } ELSE IF $first.name.first {
        $first.name.first;
      } ELSE {
        NONE;
      };
    };

-- VALUE runs on resource writes; related writes must also refresh the stored key.
  DEFINE EVENT IF NOT EXISTS refresh_primary_creator_on_edge ON created
    WHEN true
    THEN {
      IF $before.out != NONE {
        UPDATE $before.out SET primary_creator_last_name = NONE;
      };
      IF $after.out != NONE
      AND $after.out != $before.out {
        UPDATE $after.out SET primary_creator_last_name = NONE;
      };
    };
  DEFINE EVENT IF NOT EXISTS refresh_primary_creator_on_name ON person
    WHEN $before.name != $after.name
    THEN {
      LET $person = $before.id ?? $after.id;
      UPDATE (
        SELECT VALUE out
        FROM created
        WHERE in = $person
      ) SET
        primary_creator_last_name = NONE;
    };
  DEFINE INDEX IF NOT EXISTS resource_primary_creator_last_name ON resource FIELDS primary_creator_last_name;

-- Track completion separately: creator-less resources legitimately have no key.
  IF !record::exists(oeuvre_migration:primary_creator_last_name) {
    UPDATE resource SET primary_creator_last_name = NONE;
    CREATE oeuvre_migration:primary_creator_last_name;
  };
COMMIT TRANSACTION;
