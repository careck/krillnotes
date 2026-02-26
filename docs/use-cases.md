# Krillnotes Use Cases & Ideas

A living list of schema/hook ideas to explore. Add, refine, and cross off as we go.

---

I'm thinking of the use cases as a page on the krillnotes website which is a collection of downloadable templates (krillnotes export files) each with a special purpose. Each template would be presented with a screenshot of how it looks like in the app, a user guide on how to use it (the special notes and how to structure them in the tree) and a deeper discussion on how this functionality is implemented with rhai scripts.

---

## Structured Note Taking and Brainstorming

This is old school using a hierarchy of notes similar to a mind map. This is already achieved by the simple text_note, but we can make it more interesting by adding a changelog to the top level note using the on_view hook of the top level note to display a changelog of everything that was added underneath, maybe just going back the last 7 days.

# Zettelkasten Note Taking

This is an unstructured note taking techniques. This would make use of the on_save hook to set the date and maybe the first few words of the body text into the title of the note. There would be a Zettelkasten folder note (the Kasten) which keeps its children (Zettel) sorted by title.

This technique would benefit from having a native tagging system which could be used by the Zettel's on_view to search for and display links to other notes which share the same tags.

# Book Collection

There is already a book schema in the system scripts, but this would add a Library folder note, which could have a few different sorting mechanisms added as tree hooks: by title, by author, by category, by puplication date, by rating. The on_view of the library could also display titles according to different categories.

# Travel Itinirary

When planning a big trip there are a number of different connected things to consider:
- Locations have Accommodation options
- Things to See and Do are connected to a Location
- Locations are connected via Transport options, eg. Air, Train, Car, Public Transport
- Eating Out is another important thing which is tied to a Location
- On top of the management of just the data of what's there to see and do is the timing, travel dates, times when flights or trains leave and arrive

Planning a trip can be daunting, but we could make a Krillnotes Template which helps make it easier.

