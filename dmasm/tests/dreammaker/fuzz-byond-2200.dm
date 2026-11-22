// Guards: the code of a reduced fuzz case: byond mode on 515.1637, seed 2200, "515.1637w: new: result differs"
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

/proc/fz_setup()

/proc/fz_fields(datum/d)

/proc/fz_globals()

/proc/fz_intro()

/proc/fz_run()
	fz_case(172, /datum/fz/s4, "t160", 1)

/datum/fz/proc/t160(a, b = 2, c)

/datum/fz/s4/t160(a, b = 2, c)
	. = jointext(splittext_char("ÀBçd", ""), "|")
