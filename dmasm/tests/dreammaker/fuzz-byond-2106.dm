// Guards: the code of a reduced fuzz case: byond mode on 516.1688, seed 2106, "516.1688w: g: result differs"
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
	fz_call(100, "g", null, "gt97")

/proc/gt97(a, b = 2, c)
	var/regex/xr98 = regex(".", "m")
	var/xf98 = xr98.Find(jointext(splittext_char("a😀b", "l"), "|"), -2)
	. = list(., xf98, xr98.match, xr98.index, xr98.next, xr98.group, xr98.text)
