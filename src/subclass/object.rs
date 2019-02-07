// Copyright 2017-2018, The Gtk-rs Project Developers.
// See the COPYRIGHT file at the top-level directory of this distribution.
// Licensed under the MIT license, see the LICENSE file or <http://opensource.org/licenses/MIT>

//! Module that contains all types needed for creating a direct subclass of `GObject`
//! or implementing virtual methods of it.

use ffi;
use gobject_ffi;

use std::borrow::Borrow;
use std::mem;
use std::ptr;

use translate::*;
use {Object, ObjectClass, ObjectType, SignalFlags, Type, Value};

use super::prelude::*;
use super::types;

#[macro_export]
/// Macro for boilerplate of [`ObjectImpl`] implementations.
///
/// [`ObjectImpl`]: subclass/object/trait.ObjectImpl.html
macro_rules! glib_object_impl {
    () => {
        fn get_type_data(&self) -> ::std::ptr::NonNull<$crate::subclass::TypeData> {
            Self::type_data()
        }
    };
}

/// Trait for implementors of `glib::Object` subclasses.
///
/// This allows overriding the virtual methods of `glib::Object`.
pub trait ObjectImpl: 'static {
    /// Storage for the type-specific data used during registration.
    ///
    /// This is usually generated by the [`glib_object_impl!`] macro.
    ///
    /// [`glib_object_impl!`]: ../../macro.glib_object_impl.html
    fn get_type_data(&self) -> ptr::NonNull<types::TypeData>;

    /// Property setter.
    ///
    /// This is called whenever the property of this specific subclass with the
    /// given index is set. The new value is passed as `glib::Value`.
    fn set_property(&self, _obj: &Object, _id: usize, _value: &Value) {
        unimplemented!()
    }

    /// Property getter.
    ///
    /// This is called whenever the property value of the specific subclass with the
    /// given index should be returned.
    fn get_property(&self, _obj: &Object, _id: usize) -> Result<Value, ()> {
        unimplemented!()
    }

    /// Constructed.
    ///
    /// This is called once construction of the instance is finished.
    ///
    /// Should chain up to the parent class' implementation.
    fn constructed(&self, obj: &Object) {
        self.parent_constructed(obj);
    }

    /// Chain up to the parent class' implementation of `glib::Object::constructed()`.
    ///
    /// Do not override this, it has no effect.
    fn parent_constructed(&self, obj: &Object) {
        unsafe {
            let data = self.get_type_data();
            let parent_class = data.as_ref().get_parent_class() as *mut gobject_ffi::GObjectClass;

            if let Some(ref func) = (*parent_class).constructed {
                func(obj.to_glib_none().0);
            }
        }
    }
}

unsafe extern "C" fn get_property<T: ObjectSubclass>(
    obj: *mut gobject_ffi::GObject,
    id: u32,
    value: *mut gobject_ffi::GValue,
    _pspec: *mut gobject_ffi::GParamSpec,
) {
    glib_floating_reference_guard!(obj);
    let instance = &*(obj as *mut T::Instance);
    let imp = instance.get_impl();

    match imp.get_property(&from_glib_borrow(obj), (id - 1) as usize) {
        Ok(v) => {
            // We first unset the value we get passed in, in case it contained
            // any previous data. Then we directly overwrite it with our new
            // value, and pass ownership of the contained data to the C GValue
            // by forgetting it on the Rust side.
            //
            // Without this, by using the GValue API, we would have to create
            // a copy of the value when setting it on the destination just to
            // immediately free the original value afterwards.
            gobject_ffi::g_value_unset(value);
            ptr::write(value, ptr::read(v.to_glib_none().0));
            mem::forget(v);
        }
        Err(()) => eprintln!("Failed to get property"),
    }
}

unsafe extern "C" fn set_property<T: ObjectSubclass>(
    obj: *mut gobject_ffi::GObject,
    id: u32,
    value: *mut gobject_ffi::GValue,
    _pspec: *mut gobject_ffi::GParamSpec,
) {
    glib_floating_reference_guard!(obj);
    let instance = &*(obj as *mut T::Instance);
    let imp = instance.get_impl();
    imp.set_property(
        &from_glib_borrow(obj),
        (id - 1) as usize,
        &*(value as *mut Value),
    );
}

unsafe extern "C" fn constructed<T: ObjectSubclass>(obj: *mut gobject_ffi::GObject) {
    glib_floating_reference_guard!(obj);
    let instance = &*(obj as *mut T::Instance);
    let imp = instance.get_impl();

    imp.constructed(&from_glib_borrow(obj));
}

/// Definition of a property.
pub struct Property<'a>(pub &'a str, pub fn(&str) -> ::ParamSpec);

/// Extension trait for `glib::Object`'s class struct.
///
/// This contains various class methods and allows subclasses to override the virtual methods.
pub unsafe trait ObjectClassSubclassExt: Sized + 'static {
    /// Install properties on the subclass.
    ///
    /// The index in the properties array is going to be the index passed to the
    /// property setters and getters.
    fn install_properties<'a, T: Borrow<Property<'a>>>(&mut self, properties: &[T]) {
        if properties.is_empty() {
            return;
        }

        let mut pspecs = Vec::with_capacity(properties.len());

        for property in properties {
            let property = property.borrow();
            let pspec = (property.1)(property.0);
            pspecs.push(pspec);
        }

        unsafe {
            let mut pspecs_ptrs = Vec::with_capacity(properties.len());

            pspecs_ptrs.push(ptr::null_mut());

            for pspec in &pspecs {
                pspecs_ptrs.push(pspec.to_glib_none().0);
            }

            gobject_ffi::g_object_class_install_properties(
                self as *mut _ as *mut gobject_ffi::GObjectClass,
                pspecs_ptrs.len() as u32,
                pspecs_ptrs.as_mut_ptr(),
            );
        }
    }

    /// Add a new signal to the subclass.
    ///
    /// This can be emitted later by `glib::Object::emit` and external code
    /// can connect to the signal to get notified about emissions.
    fn add_signal(&mut self, name: &str, flags: SignalFlags, arg_types: &[Type], ret_type: Type) {
        unsafe {
            super::types::add_signal(
                *(self as *mut _ as *mut ffi::GType),
                name,
                flags,
                arg_types,
                ret_type,
            );
        }
    }

    /// Add a new signal with class handler to the subclass.
    ///
    /// This can be emitted later by `glib::Object::emit` and external code
    /// can connect to the signal to get notified about emissions.
    ///
    /// The class handler will be called during the signal emission at the corresponding stage.
    fn add_signal_with_class_handler<F>(
        &mut self,
        name: &str,
        flags: SignalFlags,
        arg_types: &[Type],
        ret_type: Type,
        class_handler: F,
    ) where
        F: Fn(&super::SignalClassHandlerToken, &[Value]) -> Option<Value> + Send + Sync + 'static,
    {
        unsafe {
            super::types::add_signal_with_class_handler(
                *(self as *mut _ as *mut ffi::GType),
                name,
                flags,
                arg_types,
                ret_type,
                class_handler,
            );
        }
    }

    /// Add a new signal with accumulator to the subclass.
    ///
    /// This can be emitted later by `glib::Object::emit` and external code
    /// can connect to the signal to get notified about emissions.
    ///
    /// The accumulator function is used for accumulating the return values of
    /// multiple signal handlers. The new value is passed as second argument and
    /// should be combined with the old value in the first argument. If no further
    /// signal handlers should be called, `false` should be returned.
    fn add_signal_with_accumulator<F>(
        &mut self,
        name: &str,
        flags: SignalFlags,
        arg_types: &[Type],
        ret_type: Type,
        accumulator: F,
    ) where
        F: Fn(&super::SignalInvocationHint, &mut Value, &Value) -> bool + Send + Sync + 'static,
    {
        unsafe {
            super::types::add_signal_with_accumulator(
                *(self as *mut _ as *mut ffi::GType),
                name,
                flags,
                arg_types,
                ret_type,
                accumulator,
            );
        }
    }

    /// Add a new signal with accumulator and class handler to the subclass.
    ///
    /// This can be emitted later by `glib::Object::emit` and external code
    /// can connect to the signal to get notified about emissions.
    ///
    /// The accumulator function is used for accumulating the return values of
    /// multiple signal handlers. The new value is passed as second argument and
    /// should be combined with the old value in the first argument. If no further
    /// signal handlers should be called, `false` should be returned.
    ///
    /// The class handler will be called during the signal emission at the corresponding stage.
    fn add_signal_with_class_handler_and_accumulator<F, G>(
        &mut self,
        name: &str,
        flags: SignalFlags,
        arg_types: &[Type],
        ret_type: Type,
        class_handler: F,
        accumulator: G,
    ) where
        F: Fn(&super::SignalClassHandlerToken, &[Value]) -> Option<Value> + Send + Sync + 'static,
        G: Fn(&super::SignalInvocationHint, &mut Value, &Value) -> bool + Send + Sync + 'static,
    {
        unsafe {
            super::types::add_signal_with_class_handler_and_accumulator(
                *(self as *mut _ as *mut ffi::GType),
                name,
                flags,
                arg_types,
                ret_type,
                class_handler,
                accumulator,
            );
        }
    }

    fn override_signal_class_handler<F>(&mut self, name: &str, class_handler: F)
    where
        F: Fn(&super::SignalClassHandlerToken, &[Value]) -> Option<Value> + Send + Sync + 'static,
    {
        unsafe {
            super::types::signal_override_class_handler(
                name,
                *(self as *mut _ as *mut ffi::GType),
                class_handler,
            );
        }
    }
}

unsafe impl ObjectClassSubclassExt for ObjectClass {}

unsafe impl<T: ObjectSubclass> IsSubclassable<T> for ObjectClass {
    fn override_vfuncs(&mut self) {
        unsafe {
            let klass = &mut *(self as *const Self as *mut gobject_ffi::GObjectClass);
            klass.set_property = Some(set_property::<T>);
            klass.get_property = Some(get_property::<T>);
            klass.constructed = Some(constructed::<T>);
        }
    }
}

pub trait ObjectImplExt: ObjectImpl + ObjectSubclass {
    fn signal_chain_from_overridden(
        &self,
        token: &super::SignalClassHandlerToken,
        values: &[Value],
    ) -> Option<Value> {
        unsafe {
            super::types::signal_chain_from_overridden(
                self.get_instance().as_ptr() as *mut _,
                token,
                values,
            )
        }
    }
}

impl<T: ObjectImpl + ObjectSubclass> ObjectImplExt for T {}

#[cfg(test)]
mod test {
    use super::super::super::object::ObjectExt;
    use super::super::super::subclass;
    use super::super::super::value::{ToValue, Value};
    use super::*;
    use prelude::*;

    use std::cell::RefCell;

    static PROPERTIES: [Property; 2] = [
        Property("name", |name| {
            ::ParamSpec::string(
                name,
                "Name",
                "Name of this object",
                None,
                ::ParamFlags::READWRITE,
            )
        }),
        Property("constructed", |name| {
            ::ParamSpec::boolean(
                name,
                "Constructed",
                "True if the constructed() virtual method was called",
                false,
                ::ParamFlags::READABLE,
            )
        }),
    ];

    pub struct SimpleObject {
        name: RefCell<Option<String>>,
        constructed: RefCell<bool>,
    }

    impl ObjectSubclass for SimpleObject {
        const NAME: &'static str = "SimpleObject";
        type ParentType = Object;
        type Instance = subclass::simple::InstanceStruct<Self>;
        type Class = subclass::simple::ClassStruct<Self>;

        glib_object_subclass!();

        fn type_init(type_: &mut subclass::InitializingType<Self>) {
            type_.add_interface::<DummyInterface>();
        }

        fn class_init(klass: &mut subclass::simple::ClassStruct<Self>) {
            klass.install_properties(&PROPERTIES);

            klass.add_signal(
                "name-changed",
                SignalFlags::RUN_LAST,
                &[String::static_type()],
                ::Type::Unit,
            );

            klass.add_signal_with_class_handler(
                "change-name",
                SignalFlags::RUN_LAST | SignalFlags::ACTION,
                &[String::static_type()],
                String::static_type(),
                |_, args| {
                    let obj = args[0].get::<Object>().unwrap();
                    let new_name = args[1].get::<String>().unwrap();
                    let imp = Self::from_instance(&obj);

                    let old_name = imp.name.borrow_mut().take();
                    *imp.name.borrow_mut() = Some(new_name);

                    obj.emit("name-changed", &[&*imp.name.borrow()]).unwrap();

                    Some(old_name.to_value())
                },
            );
        }

        fn new() -> Self {
            Self {
                name: RefCell::new(None),
                constructed: RefCell::new(false),
            }
        }
    }

    impl ObjectImpl for SimpleObject {
        glib_object_impl!();

        fn set_property(&self, obj: &Object, id: usize, value: &Value) {
            let prop = &PROPERTIES[id];

            match *prop {
                Property("name", ..) => {
                    let name = value.get();
                    self.name.replace(name);
                    obj.emit("name-changed", &[&*self.name.borrow()]).unwrap();
                }
                _ => unimplemented!(),
            }
        }

        fn get_property(&self, _obj: &Object, id: usize) -> Result<Value, ()> {
            let prop = &PROPERTIES[id];

            match *prop {
                Property("name", ..) => Ok(self.name.borrow().to_value()),
                Property("constructed", ..) => Ok(self.constructed.borrow().to_value()),
                _ => unimplemented!(),
            }
        }

        fn constructed(&self, obj: &Object) {
            self.parent_constructed(obj);

            assert_eq!(obj, &self.get_instance());
            assert_eq!(self as *const _, Self::from_instance(obj) as *const _);

            *self.constructed.borrow_mut() = true;
        }
    }

    #[repr(C)]
    pub struct DummyInterface {
        parent: gobject_ffi::GTypeInterface,
    }

    impl ObjectInterface for DummyInterface {
        const NAME: &'static str = "DummyInterface";

        glib_object_interface!();

        fn type_init(type_: &mut subclass::InitializingType<Self>) {
            type_.add_prerequisite::<Object>();
        }
    }

    // Usually this would be implemented on a Rust wrapper type defined
    // with glib_wrapper!() but for the test the following is sufficient
    impl StaticType for DummyInterface {
        fn static_type() -> Type {
            DummyInterface::get_type()
        }
    }

    // Usually this would be implemented on a Rust wrapper type defined
    // with glib_wrapper!() but for the test the following is sufficient
    unsafe impl<T: ObjectSubclass> IsImplementable<T> for DummyInterface {
        unsafe extern "C" fn interface_init(_iface: ffi::gpointer, _iface_data: ffi::gpointer) {}
    }

    #[test]
    fn test_create() {
        let type_ = SimpleObject::get_type();
        let obj = Object::new(type_, &[]).unwrap();

        assert!(obj.get_type().is_a(&DummyInterface::static_type()));

        assert_eq!(
            obj.get_property("constructed").unwrap().get::<bool>(),
            Some(true)
        );

        assert_eq!(obj.get_property("name").unwrap().get::<&str>(), None);
        obj.set_property("name", &"test").unwrap();
        assert_eq!(
            obj.get_property("name").unwrap().get::<&str>(),
            Some("test")
        );

        let weak = obj.downgrade();
        drop(obj);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn test_signals() {
        use std::sync::{Arc, Mutex};

        let type_ = SimpleObject::get_type();
        let obj = Object::new(type_, &[("name", &"old-name")]).unwrap();

        let name_changed_triggered = Arc::new(Mutex::new(false));
        let name_changed_clone = name_changed_triggered.clone();
        obj.connect("name-changed", false, move |args| {
            let _obj = args[0].get::<Object>().unwrap();
            let name = args[1].get::<&str>().unwrap();

            assert_eq!(name, "new-name");
            *name_changed_clone.lock().unwrap() = true;

            None
        })
        .unwrap();

        assert_eq!(
            obj.get_property("name").unwrap().get::<&str>(),
            Some("old-name")
        );
        assert!(!*name_changed_triggered.lock().unwrap());

        let old_name = obj
            .emit("change-name", &[&"new-name"])
            .unwrap()
            .unwrap()
            .get::<String>();
        assert_eq!(old_name, Some(String::from("old-name")));
        assert!(*name_changed_triggered.lock().unwrap());
    }
}
