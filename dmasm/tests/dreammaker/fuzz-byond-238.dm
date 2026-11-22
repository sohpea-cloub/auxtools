// Guards: the code of a reduced fuzz case: byond mode on 516.1689, seed 238, "515.1647: new: globals differ"
// Known: /datum/fzo/o4/proc/operator>>=  not by design: DreamMaker starts an assignment operator with `. = src` and dmasm doesn't, so one that returns nothing gives null
#define DEBUG

#define FZD6 0
#ifdef FZD_ONE
#if FZM7 > 2
#endif
#endif
var/fz_og

/datum/fzo/o4/proc/operator>>=(b)

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
	var/list/vl14 = list("a" = 1, "b" = 2)
	var/va15

/datum/fz/s7

/datum/fz/s8

/datum/fz/s9
	var/va22

var/list/gl30 = list(gn32)
var/gn31 = 256
var/gn32 = gn31 + 1
/datum/fz/New(n, t)
	fz_news++

/proc/fz_setup()

/proc/fz_fields(datum/d)

/proc/fz_globals()
	return "news=[fz_news] gl30=[fz_show(gl30, 2)] gn31=[fz_show(gn31, 2)] gn32=[fz_show(gn32, 2)]"

/proc/fz_intro()

/proc/fz_run()
	fz_case(99, /obj/fz/s11, "t96", 1)

/datum/fz/s9/s10/proc/h50(x1 = 65535, x2 = null)
	try
		try
			var/datum/xo52 = new /datum/fzo/o4(2)
		catch (var/xe52)
		try
			var/xo53 = new /datum/fzo/o4(2)
		catch (var/xe53)
	catch (var/x51)

/obj/fz/proc/h57(x1 = null, x2)

/obj/fz/s11/proc/t96(a, b = 2, c)
	var/n2 = 16777215
	var/datum/fz/s9/s10/o = new /datum/fz/s9/s10(7)
	var/datum/fz/s7/p = new /datum/fz/s7
	o.va15 = ((text2path("/datum/fz") ? call(text2path("/proc/gh55"))(src.h57(p.vl14["a"], o.h50((new /datum/fz/s8(isinf(1)) ? n2 : 0x1F), p.vl14)), "The [FZD6]x=[a] \n([gl30]", json_decode("{\"a\":1,\"b\":\[2]}")) : p.va15) ? o.va22 : (/obj/fz && (p.va15 && p.va15)))
	try
		fz_og &= new /datum/fzo(4)
	catch (var/xe97)
