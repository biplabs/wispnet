#include "my_application.h"
#include <flutter_linux/flutter_linux.h>
#include <libayatana-appindicator/app-indicator.h>
#include "flutter/generated_plugin_registrant.h"

struct _MyApplication {
  GtkApplication parent_instance;
  char** dart_entrypoint_arguments;
  GtkWindow* window;
  FlMethodChannel* channel;
  AppIndicator* tray;
  GtkWidget* status;
  gboolean background;
  GQueue* links;
};
G_DEFINE_TYPE(MyApplication, my_application, GTK_TYPE_APPLICATION)

static void show_window(GtkMenuItem*, gpointer data) {
  MyApplication* self = MY_APPLICATION(data);
  gtk_widget_show(GTK_WIDGET(self->window));
  gtk_window_present(self->window);
}
static void tray_activate(AppIndicator*, gint, gint, gpointer data) {
  show_window(nullptr, data);
}
static void exit_finished(GObject* source, GAsyncResult* result, gpointer data) {
  MyApplication* self = MY_APPLICATION(data);
  g_autoptr(GError) error = nullptr;
  g_autoptr(FlMethodResponse) response = fl_method_channel_invoke_method_finish(
      FL_METHOD_CHANNEL(source), result, &error);
  if (error || !FL_IS_METHOD_SUCCESS_RESPONSE(response)) {
    show_window(nullptr, self);
    GtkWidget* dialog = gtk_message_dialog_new(self->window, GTK_DIALOG_MODAL,
        GTK_MESSAGE_ERROR, GTK_BUTTONS_CLOSE, "Could not stop Wispnet. Please try again.");
    g_signal_connect_swapped(dialog, "response", G_CALLBACK(gtk_widget_destroy), dialog);
    gtk_widget_show(dialog);
  } else {
    g_application_quit(G_APPLICATION(self));
  }
  g_object_unref(self);
}
static void quit_app(GtkMenuItem*, gpointer data) {
  MyApplication* self = MY_APPLICATION(data);
  fl_method_channel_invoke_method(self->channel, "exit", nullptr, nullptr,
      exit_finished, g_object_ref(self));
}
static gboolean hide_window(GtkWidget* window, GdkEvent*, gpointer) {
  gtk_widget_hide(window);
  return TRUE;
}
static void first_frame_cb(MyApplication* self, FlView*) {
  if (!self->background) show_window(nullptr, self);
}
static void shell_call(FlMethodChannel*, FlMethodCall* call, gpointer data) {
  MyApplication* self = MY_APPLICATION(data);
  if (strcmp(fl_method_call_get_name(call), "takeLink") == 0) {
    g_autofree gchar* link = static_cast<gchar*>(g_queue_pop_head(self->links));
    g_autoptr(FlValue) value = link ? fl_value_new_string(link) : fl_value_new_null();
    fl_method_call_respond_success(call, value, nullptr);
  } else if (strcmp(fl_method_call_get_name(call), "status") == 0) {
    FlValue* args = fl_method_call_get_args(call);
    FlValue* text = fl_value_lookup_string(args, "text");
    gtk_menu_item_set_label(GTK_MENU_ITEM(self->status), text ? fl_value_get_string(text) : "Wispnet");
    app_indicator_set_title(self->tray, text ? fl_value_get_string(text) : "Wispnet");
    fl_method_call_respond_success(call, nullptr, nullptr);
  } else {
    fl_method_call_respond_not_implemented(call, nullptr);
  }
}
static void my_application_activate(GApplication* application) {
  MyApplication* self = MY_APPLICATION(application);
  if (self->window) { show_window(nullptr, self); return; }
  self->window = GTK_WINDOW(gtk_application_window_new(GTK_APPLICATION(application)));
  gtk_window_set_title(self->window, "Wispnet");
  gtk_window_set_default_size(self->window, 1000, 760);
  gtk_widget_set_size_request(GTK_WIDGET(self->window), 360, 440);
  g_signal_connect(self->window, "delete-event", G_CALLBACK(hide_window), self);
  g_autoptr(FlDartProject) project = fl_dart_project_new();
  fl_dart_project_set_dart_entrypoint_arguments(project, self->dart_entrypoint_arguments);
  FlView* view = fl_view_new(project);
  GdkRGBA background;
  gdk_rgba_parse(&background, "#070b14");
  fl_view_set_background_color(view, &background);
  gtk_widget_show(GTK_WIDGET(view));
  gtk_container_add(GTK_CONTAINER(self->window), GTK_WIDGET(view));
  fl_register_plugins(FL_PLUGIN_REGISTRY(view));
  g_autoptr(FlStandardMethodCodec) codec = fl_standard_method_codec_new();
  self->channel = fl_method_channel_new(fl_engine_get_binary_messenger(fl_view_get_engine(view)), "app.wispnet/desktop", FL_METHOD_CODEC(codec));
  fl_method_channel_set_method_call_handler(self->channel, shell_call, self, nullptr);
  GtkWidget* menu = gtk_menu_new();
  self->status = gtk_menu_item_new_with_label("Wispnet");
  gtk_widget_set_sensitive(self->status, FALSE);
  gtk_menu_shell_append(GTK_MENU_SHELL(menu), self->status);
  GtkWidget* quit = gtk_menu_item_new_with_label("Exit");
  g_signal_connect(quit, "activate", G_CALLBACK(quit_app), self);
  gtk_menu_shell_append(GTK_MENU_SHELL(menu), quit);
  gtk_widget_show_all(menu);
  g_autofree gchar* executable = g_file_read_link("/proc/self/exe", nullptr);
  g_autofree gchar* directory = g_path_get_dirname(executable);
  g_autofree gchar* icon = g_build_filename(directory, "icon.png", nullptr);
  self->tray = APP_INDICATOR(g_object_new(APP_INDICATOR_TYPE, "id", "wispnet",
      "icon-name", icon, "category", "ApplicationStatus", nullptr));
  g_signal_connect(self->tray, "activate", G_CALLBACK(tray_activate), self);
  app_indicator_set_status(self->tray, APP_INDICATOR_STATUS_ACTIVE);
  app_indicator_set_menu(self->tray, GTK_MENU(menu));
  g_signal_connect_swapped(view, "first-frame", G_CALLBACK(first_frame_cb), self);
  gtk_widget_realize(GTK_WIDGET(view));
  gtk_widget_grab_focus(GTK_WIDGET(view));
}
static void my_application_open(GApplication* application, GFile** files, gint count, const gchar*) {
  MyApplication* self = MY_APPLICATION(application);
  for (int i = 0; i < count; ++i) {
    g_autofree gchar* uri = g_file_get_uri(files[i]);
    if (g_str_has_prefix(uri, "wispnet://") && strlen(uri) <= 256 && g_queue_get_length(self->links) < 16)
      g_queue_push_tail(self->links, g_strdup(uri));
  }
  g_application_activate(application);
}
static gboolean my_application_local_command_line(GApplication* application, gchar*** arguments, int* exit_status) {
  MyApplication* self = MY_APPLICATION(application);
  self->dart_entrypoint_arguments = g_strdupv(*arguments + 1);
  for (char** arg = *arguments; *arg; ++arg) {
    if (!strcmp(*arg, "--tray") || !strcmp(*arg, "--background")) self->background = TRUE;
  }
  g_autoptr(GError) error = nullptr;
  if (!g_application_register(application, nullptr, &error)) {
    g_warning("Failed to register: %s", error->message); *exit_status = 1; return TRUE;
  }
  gboolean opened = FALSE;
  for (char** arg = *arguments + 1; *arg; ++arg) {
    if (g_str_has_prefix(*arg, "wispnet://")) {
      g_autoptr(GFile) file = g_file_new_for_uri(*arg);
      GFile* files[] = {file};
      g_application_open(application, files, 1, "");
      opened = TRUE;
    }
  }
  if (!opened) g_application_activate(application);
  *exit_status = 0;
  return TRUE;
}
static void my_application_dispose(GObject* object) {
  MyApplication* self = MY_APPLICATION(object);
  g_clear_object(&self->tray);
  g_clear_object(&self->channel);
  g_clear_pointer(&self->dart_entrypoint_arguments, g_strfreev);
  if (self->links) { g_queue_free_full(self->links, g_free); self->links = nullptr; }
  G_OBJECT_CLASS(my_application_parent_class)->dispose(object);
}
static void my_application_class_init(MyApplicationClass* klass) {
  G_APPLICATION_CLASS(klass)->open = my_application_open;
  G_APPLICATION_CLASS(klass)->activate = my_application_activate;
  G_APPLICATION_CLASS(klass)->local_command_line = my_application_local_command_line;
  G_OBJECT_CLASS(klass)->dispose = my_application_dispose;
}
static void my_application_init(MyApplication* self) { self->links = g_queue_new(); }
MyApplication* my_application_new() {
  g_set_prgname("wispnet");
  return MY_APPLICATION(g_object_new(my_application_get_type(), "application-id", APPLICATION_ID, "flags", G_APPLICATION_HANDLES_OPEN, nullptr));
}
