// Guards: the fuzzer's fixed harness, which the other fuzz cases declare without bodies
// Builds: 516.1648-
#define DEBUG

var/fz_news = 0
var/fz_budget = 0
var/list/fz_out = list()
var/list/fz_logs = list()
var/list/fz_old = list()

/proc/fz_show(v, depth = 0)
	if (isnull(v))
		return "null"
	if (isnum(v))
		return num2text(v, 12)
	if (istext(v))
		return "\"[v]\""
	if (++fz_budget > 300)
		return "~"
	if (islist(v))
		if (depth > 3)
			return "list(..)"
		var/list/l = v
		var/keyed = 0
		keyed = istype(l, /alist)
		var/list/parts = list()
		for (var/k in l)
			if (length(parts) >= 30)
				parts += "..."
				break
			var/text = fz_show(k, depth + 1)
			if (keyed || istext(k) || istype(k, /datum) || ispath(k))
				var/val = l[k]
				if (!isnull(val))
					text += "=[fz_show(val, depth + 1)]"
			parts += text
		if (keyed)
			fz_sort(parts)
		return "[keyed ? "alist" : "list"]([jointext(parts, ",")])"
	if (istype(v, /datum))
		var/datum/d = v
		if (depth <= 1)
			return "[d.type]{[fz_fields(d)]}"
		return "[d.type]"
	if (ispath(v))
		return "[v]"
	return "?[v]"

/proc/fz_log(tag, v)
	fz_budget = 0
	fz_logs += fz_line("[tag]: [fz_show(v)]")

/proc/fz_line(t)
	return replacetext(replacetext(t, "\\", "\\\\"), "\n", "\\n")

/proc/fz_sort(list/l)
	for (var/i in 2 to length(l))
		var/j = i
		while (j > 1 && sorttextEx(l[j - 1], l[j]) < 0)
			l.Swap(j - 1, j)
			j--
	return l

/proc/fz_call(id, dom, datum/d, name)
	var/r
	rand_seed(id)
	fz_budget = 0
	try
		if (d)
			if (id % 4)
				r = call(d, name)(id, id * 2 + 1, "c[id]")
			else
				r = call(d, name)(id)
		else
			if (id % 4)
				r = call(text2path("/proc/[name]"))(id, id * 2 + 1, "c[id]")
			else
				r = call(text2path("/proc/[name]"))(id)
		r = fz_show(r)
	catch (var/e)
		var/exception/ex = e
		if (istype(ex))
			r = "runtime: [ex.name] @[ex.file]:[ex.line]"
		else
			r = "thrown: [fz_show(e)]"
	fz_budget = 0
	var/fields = d ? fz_fields(d) : ""
	fz_budget = 0
	fz_out += fz_line("[id] [dom] [name]: [r] | [fields] | [fz_globals()]")

/proc/fz_case(id, t, name, old)
	var/datum/d
	if (ispath(t, /atom))
		d = new t(null, id, "n[id]")
	else
		d = new t(id, "n[id]")
	fz_call(id, "new", d, name)
	if (old)
		fz_call(id, "old", fz_old["[t]"], name)

/proc/fz_ti(t, list/names, list/procs, datum/old)
	var/datum/d = old || new t
	var/list/parts = list("[old ? "old " : ""][t] parent=[d.parent_type]")
	fz_budget = 0
	for (var/n in names)
		try
			if (copytext(n, 1, 2) == "!")
				n = copytext(n, 2)
				parts += "[n]:/[issaved(d.vars[n])]"
			else
				parts += "[n]:[fz_show(initial(d.vars[n]), 2)]/[issaved(d.vars[n])]"
		catch
			parts += "[n]:!"
	for (var/n in procs)
		parts += "[n]():[hascall(d, n)]"
	if (istype(d, /atom))
		var/atom/a = d
		var/list/vs = list()
		for (var/vb in a.verbs)
			vs += "[vb:name]/[vb:category]/[vb:desc]"
		parts += "verbs=[jointext(fz_sort(vs), ";")]"
	var/list/subs = list()
	for (var/s in typesof(t))
		subs += "[s]"
	parts += "types=[jointext(fz_sort(subs), ";")]"
	fz_budget = 0
	parts += "fields=[fz_fields(d)]"
	fz_out += fz_line(jointext(parts, " "))

/proc/fz_main()
	while (!fexists("go"))
		sleep(1)
	fz_news = 0
	fz_intro()
	fz_run()
	sleep(3)
	fz_out += fz_logs
	fz_out += "end"
	fdel("results.txt")
	text2file(jointext(fz_out, "\n"), "results.txt")
	shutdown()

/world/New()
	..()
	fz_setup()
	text2file("ok", "ready")
	spawn (0)
		fz_main()

/datum/fz/s4

/datum/fz/s4/s5

/proc/fz_setup()

/proc/fz_fields(datum/d)

/proc/fz_globals()

/proc/fz_intro()
	fz_ti(/datum/fz/s4/s5, list("vn8", "vt9", "!vl10", "va11", "vo12", "vn13", "vn14"), list("t105"), null)

/proc/fz_run()
