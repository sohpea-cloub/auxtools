// Guards: the code of a reduced fuzz case: byond mode on 516.1688, seed 4, "515.1647: new: runtime error only here: bad resource file"
#define DEBUG

var/fz_news = 0
var/fz_budget = 0
var/list/fz_out = list()
var/list/fz_logs = list()
var/list/fz_old = list()
/proc/fz_show(v, depth = 0)
/proc/fz_log(tag, v)
/proc/fz_line(t)
/proc/fz_sort(list/l)
/proc/fz_call(id, dom, datum/d, name)
/proc/fz_case(id, t, name, old)
/proc/fz_ti(t, list/names, list/procs, datum/old)
/proc/fz_main()
/world/New()

/datum/fz/s5/s6
	var/const/vn20 = 65535

/proc/fz_setup()

/proc/fz_fields(datum/d)

/proc/fz_globals()

/proc/fz_intro()

/proc/fz_run()
	fz_case(179, /datum/fz/s5/s6, "t177", 1)

/datum/fz/proc/h42(x1)
	usr << browse_rsc("UPPER lower")

/datum/fz/s5/s6/proc/t177(a, b = 2, c)
	if (vn20) return call(src, /datum/fz/proc/h42)()
