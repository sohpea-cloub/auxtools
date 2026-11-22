// Guards: the code of a reduced fuzz case: byond mode on 516.1688, seed 2265, "516.1688w: new: result differs"
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

/obj/fz
	var/vt25 = "x"

var/ga31

/proc/fz_setup()

/proc/fz_fields(datum/d)

/proc/fz_globals()

/proc/fz_intro()

/proc/fz_run()
	fz_case(66, /obj/fz/s10, "vb61", 1)

/obj/fz/s10/verb/vb61(a as num, b as num, c as text)
	var/list/M = list("a" = -1, "b" = "")
	if ((vt25 + "[M]\t")) return text2num("\improper ["A,b,c"] a [src].[-1])[ga31]\t")
