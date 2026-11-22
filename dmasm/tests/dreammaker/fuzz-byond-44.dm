// Guards: the code of a reduced fuzz case: byond mode on 516.1688, seed 44, "515.1647: new: globals differ"
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

/obj/fz/s7/s8

var/list/gl26
var/gn27 = 1e3
var/gn28 = 0.001
var/gn29
var/list/gl30 = list()
var/ga31 = "k=1&j=2"

/proc/fz_setup()

/proc/fz_fields(datum/d)

/proc/fz_globals()
	return "news=[fz_news] gl26=[fz_show(gl26, 2)] gn27=[fz_show(gn27, 2)] gn28=[fz_show(gn28, 2)] gn29=[fz_show(gn29, 2)] gl30=[fz_show(gl30, 2)] ga31=[fz_show(ga31, 2)]"

/proc/fz_intro()

/proc/fz_run()
	fz_case(113, /datum/fz/s5, "t110", 1)

/datum/fz/s5/proc/t110(a, b = 2, c)
	var/obj/fz/o = new /obj/fz
	gn29 = length(o.vars)
