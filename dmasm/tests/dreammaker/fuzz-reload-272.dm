// Guards: the code of a reduced fuzz case: reload mode on 516.1688, seed 272, "type info differs: vnN"
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

/datum/fz/s4

/datum/fz/s4/s5

/proc/fz_setup()

/proc/fz_fields(datum/d)

/proc/fz_globals()

/proc/fz_intro()
	fz_ti(/datum/fz/s4/s5, list("vn8", "vt9", "!vl10", "va11", "vo12", "vn13", "vn14"), list("t105"), null)

/proc/fz_run()
