## Here are some things to improve 

✅ DONE! Refactor the scripting engine so that the scripts aren't read in the schemaregistry, but that they are called from a scriptregistry. The schemaregistry should only know about schemas
✅ DONE! Refactor the hook registry so that it lives outside the schema registry and has its own spac
✅ DONE! change the add note behavior so that the new note immediately is selected and displays in the edit mode
✅ DONE! include a flag in the rhai schema which indicates whether a field is shown in view and/or edit mode. Also include a flag which determines whether the note title can be edited. For instance when editing the contact note the title should'nt be shown in the editor for editing as it is being calculated by the on_save script
✅ DONE! fix the default view so that it is more compressed, eg. align all field names and values in one line unless the values are too long. maybe use a table like layout so that all titles align underneath and all values align to the right. Also if a field has no value or is empty then do not display it at all.

[x] tree navigation via keyboard, eg. arrow keys, down goes down the siblings, right opens a parent node, left closes the parent node, up goes up the siblings, each selected note is displayed immediately in the view panel (already implemented), but enter key opens the edit mode for the selected note.

[ ] make a view hook which is called when a note is displayed in the view panel, the hook function (in a rhai script) will return some templated html code to display the note. The view should also have access to all children of the note and query and display their content as well. This would allow for displaying notes with nested child notes.