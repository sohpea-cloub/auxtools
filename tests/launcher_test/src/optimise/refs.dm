// Writes the ref of one thing of each kind the refs pass adds strings for.
#define OUT(X) text2file(X, "result.txt")

/world/New()
	..()
	fdel("result.txt")
	maxx = 1
	maxy = 1
	maxz = 1
	var/list/things = list("turf" = locate(1, 1, 1), "obj" = new /obj, "mob" = new /mob, "image" = new /image, "list" = list(1), "datum" = new /datum)
	for(var/kind in things)
		var/thing = things[kind]
		OUT("[kind] \ref[thing]")
	OUT("done")
	shutdown()
