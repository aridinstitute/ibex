-- Diagnostic fixture: the pre-optimization computed field and per-write events.
DEFINE FIELD OVERWRITE primary_creator_last_name ON resource TYPE option<string>
  VALUE {
    LET $resource = $this.id;
    LET $first = (
      SELECT order, credited_as, in.name AS name FROM created
      WHERE out = $resource ORDER BY order LIMIT 1
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
DEFINE EVENT refresh_primary_creator_on_edge ON created WHEN true THEN {
  IF $before.out != NONE {
    UPDATE $before.out SET primary_creator_last_name = NONE;
  };
  IF $after.out != NONE AND $after.out != $before.out {
    UPDATE $after.out SET primary_creator_last_name = NONE;
  };
};
DEFINE EVENT refresh_primary_creator_on_name ON person
  WHEN $before.name != $after.name THEN {
    LET $person = $before.id ?? $after.id;
    UPDATE (SELECT VALUE out FROM created WHERE in = $person)
      SET primary_creator_last_name = NONE;
  };
DEFINE INDEX IF NOT EXISTS resource_primary_creator_last_name ON resource FIELDS primary_creator_last_name;
