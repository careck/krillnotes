# This is the worksheet of tasks we will be implementing step by step
#
# I will feed you one of these tasks at a time, but when you're done,
# please find it in this file and mark it as done in the same way the other tasks are already marked as done.

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

✅ DONE! add a search bar at the very top above the tree. when typing anything it will find any note with that text anywhere in its fields and display them in a drop down. once you click on a note in the dropdown it will open up the note for editing and show it in the tree. If the note was previous not visible because its parent was collapsed then its parent and all grandparents will also be shown.

✅ DONE! make a view which shows the operations log of unsynced operations. This would be a flat list of operations with date and time in the left hand column and the operation type (create, update, delete) in the right hand column. The user should be able to filter the list by operation type or by date range. There should also be a button to purge the log from the database to compress it.

✅ DONE! make an export feature which stores the whole workspace as a zip file with a json file for the notes data and all user scripts as separate .rhai files in the same folder. This would allow for sharing of workspaces between users without having to worry about syncing their entire database. Also allow for importing of other people's workspaces into a new workspace from a zip file of the export format. When exporting a workspace remove all operation logs, as these are not relevant for an export file. The import/export actions should be available from the File menu.

✅ DONE! enable node drag and drop operations to change their position among their siblings as well as move the note and all its children up or down in the tree structure (basically change the parent node). Special consideration should be given if a child node should be moved to a root node as then you don't have a parent node to point to for dropping purposes. Move actions should be via mouse drag and drop.

✅ DONE! remove the save dialog from "new" and open file dialog from "open". Instead add a settings dialog and data structure where the user can set the default workspace directory. All new workspaces will then automatically be created in this directory. Loading a workspace can have a new dialog which lists all available workspaces in that directory and allows for loading of any existing workspace (which is not currently open). There should be default workspace directory which makes sense depending on the operating system (e.g. ~/Documents/Krillnotes). The settings should  be stored in a location which makes sense for each OS (e.g. ~/.krillnotes/settings.json). Remember that this will also change the way Import works, as it can now create the new workspace directly in the default directory instead of having to ask for a path.

✅ DONE! let's remove the difference between user scripts and system scripts! move all the existing user scripts into the system_scripts folder, and load them all on startup of a new workspace as if they are user scripts. this way a user can even change the initial text note schema. Switch all scripts on at the start, and the user can then switch off the ones they don't need. Also add a "delete" button next to a user script so that any scripts which the user doesn't need can also be removed completely from the workspace. System scripts will only be loaded when a workspace is created, not when loaded as by then the workspace has its own scripts.

✅ DONE! allow a note schema to define which schema a parent may have to add this note. This would be useful for creating the contactfolder, where each contact note only shows up as an option on add note if the selected note is of that allowed type and the operation is "add as a child". This would need to make the add note dialog more dynamic so that the note type dropdown changes according to a) the selected note type and b) "as child" operation. In the schema definition the allowed parent schema type should be a string array of allowed parent schema types. When moving a note, the new parent note where the note would be dropped needs to be checked against the allowed parent schema types and in case it doesn't match it should be a no op.

✅ DONE! we just implemented a feature where we allowed a child note to determine which parent type it can be placed under. Now we also do the opposite ... let a parent note decide which type of notes can be placed under it as children! So if a schema has this setting (which is an array of possible children types) then only those types can be placed under it and only those will be in the dropdown when adding a child node. Now there can be a weird situation where a parent specifies a child schema, but the child schema does not allow that parent schema! In this case, the child wins!

✅ DONE! make a view hook which is called when a note is displayed in the view panel, the hook function (in a rhai script) will return some templated html code to display the note. The view should also have access to all children of the note and query and display their content as well. This would allow for displaying notes with nested child notes. An example is the ContactFolder in the system_scripts which only has contact note children. When viewing a contactfolder note then the view could show a tabular view of the contact children. Please explore options, as I want it to be html like, but also super simple to define various different types of views.

✅ DONE! create a new File system menu and move load and save workspace to this. Move settings to the Krillnotes menu.

✅ DONE! add the view function link_to(note) to the view commands. As part of this feature you will also need to implement a note view history and back button functionality, so that the user can go back to the original viewed note after following a link from another note.

✅ DONE! I know how to manually build a MacOS app bundle using tauri build, but I would like to automate this using github actions. Please suggest ways to: 1. kick this off by either setting a release tag or pushing a tag to the repo; 2. create an artifact automatically for the release for windows, macos and linux.

✅ DONE! enable markdown rendering for all textarea fields. The default view should automatically render the value as markdown, however when accessing the value via the API in a rhai script, the value should be returned as plain text. Add a markdown render view command for rhai scripting.

[ ] I thought about the design decision to have on_save() and on_view() hooks outside the schema definition and I think it causes more trouble than it's worth. I thought it would enable a user to override existing functionality, but since we have eliminated the distinction between system scripts and user scripts, a user can just edit existing schema and hooks directly without the need to override anything. In fact, having these hooks outside the schema now makes the headache of load and execution order! Which on_view() to call when there are two registered for the same schema in different scripts? It's even worse for on_save()! I would like to move the on_save() and on_view() hooks back into the schema definition and change their rhai method signature to only accept a note, since the schema is already known through the encapsulation. This would make the schema definition more self-contained and eliminate any ambiguities of which hook to call when.

[ ] in edit mode, if the last field in a schema is a textarea field, then display the textarea input field with a height so that it stretches all the way to the bottom of the window.

[ ] Add encryption to the database file using SQLCipher. All new workspaces should be encrypted this way, but old workspaces should be opened with a warning and a dialog to add a password to encrypt it. Try to use OS keychain management if possible.

[ ] Exports should stay in clear text, but offer the option to encrypt the zip file with a password. On import the app should recognise if a zip is encrypted and prompt the user for the password to decrypt before opening it.
