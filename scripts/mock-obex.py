#!/usr/bin/env python3
"""A mock obexd (BlueZ's file transfer service) on the session bus, for tests and screenshots.

Sending: Client1.CreateSession and ObjectPush1.SendFile make transfers that finish in about two
seconds. Receiving: org.bluez.obex.Mock.Push(address, name, size) asks the registered agent with
AuthorizePush, as obexd does when a phone sends a file, and writes `size` bytes where it answers.
Mock.PushBatch sends several files over one connection, and Mock.Fail makes a file being sent
fail, as when the phone declines it.

    DBUS_SESSION_BUS_ADDRESS=... python3 scripts/mock-obex.py
"""
import os

import dbus
import dbus.service
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib

ROOT = "/org/bluez/obex"
PROPS = "org.freedesktop.DBus.Properties"
CACHE = os.path.join(os.environ.get("XDG_CACHE_HOME") or os.path.expanduser("~/.cache"), "obexd")


class Object(dbus.service.Object):
    """An object with properties on one interface, and PropertiesChanged."""

    def __init__(self, bus, path, iface, props):
        super().__init__(bus, path)
        self.iface = iface
        self.props = props

    @dbus.service.method(PROPS, in_signature="ss", out_signature="v")
    def Get(self, iface, name):
        return self.props[name]

    @dbus.service.method(PROPS, in_signature="s", out_signature="a{sv}")
    def GetAll(self, iface):
        return self.props if iface == self.iface else {}

    @dbus.service.signal(PROPS, signature="sa{sv}as")
    def PropertiesChanged(self, iface, changed, invalidated):
        pass

    def set(self, **changed):
        self.props.update(changed)
        self.PropertiesChanged(self.iface, changed, [])


class Transfer(Object):
    def __init__(self, bus, path, session, name, size, filename, client=False):
        super().__init__(bus, path, "org.bluez.obex.Transfer1", {
            "Status": "queued", "Name": name, "Size": dbus.UInt64(size), "Transferred": dbus.UInt64(0),
            "Filename": filename, "Session": dbus.ObjectPath(session), "Type": "",
        })
        # obexd's client drops a sent file's transfer as soon as it ends, in the same call that
        # sets its last Status (client/session.c: session_terminate_transfer, pending_request_free,
        # obc_transfer_unregister).
        self.client = client
        self.then = None

    @dbus.service.method("org.bluez.obex.Transfer1")
    def Cancel(self):
        self.fail()

    def fail(self):
        """The phone declined the file, went out of range, or the transfer was cancelled."""
        self.end("error")

    def end(self, status):
        self.set(Status=status)
        if self.client:
            self.remove_from_connection()
        then, self.then = self.then, None
        if then:
            then()

    def run(self, then=None):
        """Moves the bytes along over about two seconds."""
        self.then = then
        self.set(Status="active")
        size = int(self.props["Size"])

        def step():
            if self.props["Status"] != "active":
                return False
            done = min(size, int(self.props["Transferred"]) + max(1, size // 10))
            self.set(Transferred=dbus.UInt64(done))
            if done >= size:
                self.end("complete")
                return False
            return True

        GLib.timeout_add(200, step)


class Session(Object):
    def __init__(self, bus, path, destination):
        super().__init__(bus, path, "org.bluez.obex.Session1", {"Destination": destination, "Target": "opp"})
        self.count = 0
        self.queue = []

    @dbus.service.method("org.bluez.obex.ObjectPush1", in_signature="s", out_signature="oa{sv}")
    def SendFile(self, filename):
        size = os.path.getsize(filename)
        path = f"{self.__dbus_object_path__}/transfer{self.count}"
        self.count += 1
        t = Transfer(self.connection, path, self.__dbus_object_path__, os.path.basename(filename), size, filename,
                     client=True)
        self.queue.append(t)
        if len(self.queue) == 1:
            t.run(self.next)
        return dbus.ObjectPath(path), t.props

    def next(self):
        self.queue.pop(0)
        if self.queue:
            self.queue[0].run(self.next)

    def find(self, path):
        return next((t for t in self.queue if t.__dbus_object_path__ == path), None)


class Obex(dbus.service.Object):
    def __init__(self, bus):
        super().__init__(bus, ROOT)
        self.bus = bus
        self.agent = None
        self.sessions = 0
        self.objects = []

    @dbus.service.method("org.bluez.obex.AgentManager1", in_signature="o", sender_keyword="sender")
    def RegisterAgent(self, path, sender=None):
        self.agent = (sender, path)

    @dbus.service.method("org.bluez.obex.AgentManager1", in_signature="o")
    def UnregisterAgent(self, path):
        self.agent = None

    @dbus.service.method("org.bluez.obex.Client1", in_signature="sa{sv}", out_signature="o")
    def CreateSession(self, destination, args):
        path = f"{ROOT}/client/session{self.sessions}"
        self.sessions += 1
        self.objects.append(Session(self.bus, path, destination))
        return dbus.ObjectPath(path)

    @dbus.service.method("org.bluez.obex.Client1", in_signature="o")
    def RemoveSession(self, path):
        for o in self.objects:
            if o.__dbus_object_path__ == path:
                # obexd ends the files still queued or being sent (obc_session_shutdown).
                queue, o.queue = getattr(o, "queue", []), []
                for t in queue:
                    t.then = None
                    t.fail()
                o.remove_from_connection()
        self.objects = [o for o in self.objects if o.__dbus_object_path__ != path]

    @dbus.service.method("org.bluez.obex.Mock", in_signature="sst")
    def Push(self, address, name, size):
        """A phone sends `name`: ask the agent where to put it."""
        session = f"{ROOT}/server/session{self.sessions}"
        self.sessions += 1
        self.objects.append(Session(self.bus, session, address))
        t = Transfer(self.bus, f"{session}/transfer0", session, name, size, "")
        self.objects.append(t)
        if not self.agent:
            t.set(Status="error")
            return
        agent = self.bus.get_object(self.agent[0], self.agent[1], introspect=False)

        def reply(path):
            os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
            with open(path, "wb") as f:
                f.write(b"\0" * size)
            t.set(Filename=path)
            t.run()

        agent.AuthorizePush(dbus.ObjectPath(t.__dbus_object_path__), dbus_interface="org.bluez.obex.Agent1",
                            reply_handler=reply, error_handler=lambda e: t.set(Status="error"), timeout=120)

    @dbus.service.method("org.bluez.obex.Mock", in_signature="o")
    def Fail(self, path):
        """The phone declines the file being sent to it (or goes out of range)."""
        for o in self.objects:
            t = o.find(path) if isinstance(o, Session) else None
            if t:
                t.fail()

    @dbus.service.method("org.bluez.obex.Mock", in_signature="sa(st)iu")
    def PushBatch(self, address, files, abort_at, linger):
        """A phone sends several files over one connection, as Android does when sharing a few.

        As obexd 5.86 and later: the connection has ONE transfer object, used for every file
        (plugins/opp.c: opp_connect registers it, opp_disconnect unregisters it). For each file
        its Status goes queued (opp_chkput), active, then complete, or error when the phone
        aborts (opp_reset), and Filename is there only while the file is written (obex.c
        os_reset_session frees the path, and removes an aborted file). The phone aborts file
        number `abort_at` half-way (-1: none), and disconnects `linger` ms after the last file.
        Each file is filled with its number ("1", "2", ...), so they can be told apart.
        """
        session = f"{ROOT}/server/session{self.sessions}"
        self.sessions += 1
        s = Session(self.bus, session, address)
        t = Transfer(self.bus, f"{session}/transfer0", session, "", 0, "")
        del t.props["Filename"]
        self.objects += [s, t]
        files = [(str(n), int(size)) for n, size in files]
        agent = self.bus.get_object(self.agent[0], self.agent[1], introspect=False)

        def disconnect():
            t.remove_from_connection()
            s.remove_from_connection()
            return False

        def start(i):
            t.props.pop("Filename", None)
            if i >= len(files):
                GLib.timeout_add(linger, disconnect)
                return
            name, size = files[i]
            t.set(Status="queued", Name=name, Size=dbus.UInt64(size), Transferred=dbus.UInt64(0))

            def abort(path):
                os.remove(path)
                del t.props["Filename"]
                t.set(Status="error")
                GLib.timeout_add(linger, disconnect)
                return False

            def reply(path):
                os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
                aborted = i == abort_at
                with open(path, "wb") as f:  # O_WRONLY | O_CREAT | O_TRUNC, as obex_put_stream_start
                    f.write(str(i + 1).encode() * (size // 2 if aborted else size))
                t.set(Filename=path)
                if aborted:
                    t.set(Status="active", Transferred=dbus.UInt64(size // 2))
                    GLib.timeout_add(400, abort, path)
                else:
                    t.run(lambda: start(i + 1))

            # Declined: obexd answers the phone's PUT with an error and leaves Status as it is
            # (queued) until the phone disconnects.
            agent.AuthorizePush(dbus.ObjectPath(t.__dbus_object_path__), dbus_interface="org.bluez.obex.Agent1",
                                reply_handler=reply, error_handler=lambda e: GLib.timeout_add(linger, disconnect),
                                timeout=120)

        start(0)


DBusGMainLoop(set_as_default=True)
bus = dbus.SessionBus()
name = dbus.service.BusName("org.bluez.obex", bus)
Obex(bus)
GLib.MainLoop().run()
