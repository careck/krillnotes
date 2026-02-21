## Here are some things to improve 

✅ DONE! Refactor the scripting engine so that the scripts aren't read in the schemaregistry, but that they are called from a scriptregistry. The schemaregistry should only know about schemas

✅ DONE! Refactor the hook registry so that it lives outside the schema registry and has its own space

✅ DONE! change the add note behavior so that the new note immediately is selected and displays in the edit mode

✅ DONE! include a flag in the rhai schema which indicates whether a field is shown in view and/or edit mode. Also include a flag which determines whether the note title can be edited. For instance when editing the contact note the title should'nt be shown in the editor for editing as it is being calculated by the on_save script

✅ DONE! fix the default view so that it is more compressed, eg. align all field names and values in one line unless the values are too long. maybe use a table like layout so that all titles align underneath and all values align to the right. Also if a field has no value or is empty then do not display it at all.

✅ DONE! tree navigation via keyboard, eg. arrow keys, down goes down the siblings, right opens a parent node, left closes the parent node, up goes up the siblings, each selected note is displayed immediately in the view panel (already implemented), but enter key opens the edit mode for the selected note.

✅ DONE! text fields should have only single line text inputs. Add new schema type "textarea" which is stored the exact same way as a "text", but in edit mode it is displayed in a textarea instead of a text input field. Make this change also in text_note.rhai where in the body field.

✅ DONE! we want to enable user scripts which work just like system scripts but get loaded after the system scripts are loaded. there needs to be a dialog or window which shows all user scripts and which allows to edit and reload them. Because schema are defined by user scripts, I think that user scripts should be stored IN THE workspace database file! So each workspace has their own script instance which is loaded when the workspace is opened. The script management screen is tied to a workspace and allow to LIST all user scripts, ADD a new script (edit window opens), edit an existing script, reload an existing script (happens automatically after saving a new or edited script) and delete an existing script. Deletion is dangerous as it could delete a schema definition for which we have data in the workspace, so a warning needs to be shown before deleting.

✅ DONE!  read all scripts in the system scripts folder, not just the specific ones.

✅ DONE!  make the split between tree and view/edit panel resizable view mouse dragging

✅ DONE! add a system note property which tells the tree to sort the note's children by title either asc or desc or don't sort at all (use the note position instead). This should be settable via the schema definition just like title_can_edit. 

[ ] add a search bar at the very top above the tree. when typing anything it will find any note with that text anywhere in its fields and display them in a drop down. once you click on a note in the dropdown it will open up the note for editing and show it in the tree. If the note was previous not visible because its parent was collapsed then its parent and all grandparents will also be shown.

[ ] make a view hook which is called when a note is displayed in the view panel, the hook function (in a rhai script) will return some templated html code to display the note. The view should also have access to all children of the note and query and display their content as well. This would allow for displaying notes with nested child notes.

[ ] make a view which shows the operations log of unsynced operations. This would be a flat list of operations with date and time in the left hand column and the operation type (create, update, delete) in the right hand column. The user should be able to filter the list by operation type or by date range. There should also be a button to purge the log from the database to compress it.



