var/st_total = 0

/proc/st_add_total()
	st_total++
	return st_total

var/const/ST_LIMIT = 5

/proc/st_limit()
	return ST_LIMIT

/proc/st_static_limit()
	var/static/ST_LIMIT = 7
	return ST_LIMIT

/proc/st_const_limit()
	var/const/ST_LIMIT = 8
	return ST_LIMIT

/datum/st_holder
	var/static/st_held = 1

var/ST_SCOPED = 1

/proc/st_scoped()
	var/const/ST_SCOPED = 5
	return global.ST_SCOPED

/proc/st_scoped_global()
	return ST_SCOPED

var/ST_MIXED = 1

/proc/st_mixed()
	var/static/ST_MIXED = 7
	ST_MIXED++
	return "[ST_MIXED] [global.ST_MIXED]"

/proc/st_mixed_global()
	return ST_MIXED

/datum/st_twice/proc/f()
	return 0

/datum/st_twice/f()
	return 1

/datum/st_twice/f()
	return 2

/proc/st_counter()
	var/static/st_count = 0
	st_count++
	if (/datum/st_twice::f())
		return st_count

var/const/ST_BOTH = 5

/proc/st_both_root()
	return ST_BOTH

/proc/st_both()
	var/static/ST_BOTH = 7
	return list(ST_BOTH, global.ST_BOTH)

var/st_registered = 0

/proc/st_register()
	st_registered++
	return st_registered

var/st_state = 1

/proc/st_stateful()
	var/static/st_state = st_register()
	return global.st_state + 1

var/ST_UNKNOWN = 1

/proc/st_unknown()
	var/const/ST_UNKNOWN = 5
	if (/datum/st_twice::f())
		return global.ST_UNKNOWN

/proc/st_unknown_global()
	return ST_UNKNOWN

/proc/st_branch(which)
	if (which == 1)
		var/static/st_branch_count = 0
		st_branch_count++
		return st_branch_count
	return 0

var/const/ST_RENAMED = 5

/proc/st_renamed_root()
	return ST_RENAMED

/proc/st_renamed()
	var/static/ST_RENAMED = 7
	ST_RENAMED++
	return ST_RENAMED

var/ST_DEFAULT = 1

/proc/st_defaulted(x = global.ST_DEFAULT)
	var/const/ST_DEFAULT = 5
	if (/datum/st_twice::f())
		return list(ST_DEFAULT + 1, x)

/proc/st_default_global()
	return ST_DEFAULT

/proc/st_seeded()
	var/static/st_seed = 1
	st_seed++
	return st_seed
