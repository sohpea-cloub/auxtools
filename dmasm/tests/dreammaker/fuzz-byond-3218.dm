// Guards: the code of a reduced fuzz case: byond mode on 516.1688, seed 3218, "515.1647: g: globals differ"
#define DEBUG

#define FZD3 5

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

/datum/fz
	var/static/vn15 = -1

/datum/fz/s5/s6

var/ga24
var/gn26 = 256
var/gn27 = 65535
var/ga28
/datum/fz/New(n, t)
	fz_news++

/proc/fz_setup()

/proc/fz_fields(datum/d)

/proc/fz_globals()
	return "news=[fz_news] ga24=[fz_show(ga24, 2)] gn26=[fz_show(gn26, 2)] gn27=[fz_show(gn27, 2)] ga28=[fz_show(ga28, 2)]"

/proc/fz_intro()

/proc/fz_run()
	fz_call(137, "g", null, "gt118")

/datum/fz/s7/proc/h55(x1, x2)

/proc/gt118(a, b = 2, c)
	var/s1 = "  pad  "
	var/list/L = list(256, "1", 0x1F)
	var/datum/fz/s7/p = new /datum/fz/s7(-3.25, "A,b,c")
	try
		spawn (1)
			fz_log("s120", length(L))
	catch (var/x119)
	if (p.vn15)
		s1 = "[new /datum/fz/s5((!new /datum/fz(ceil(FZD3))))]x=[new /datum/fz/s5/s6(255)] \n"
