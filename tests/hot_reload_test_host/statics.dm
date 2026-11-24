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
