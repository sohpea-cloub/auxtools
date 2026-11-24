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
