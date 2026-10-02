use std::ffi::CStr;

use ash::vk;

/// The size of the frames the app draws (RGBA8); scaled to the display's mode.
pub const FRAME_WIDTH: u32 = 1920;
pub const FRAME_HEIGHT: u32 = 1080;

const SUBRESOURCE: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::COLOR,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

const LAYERS: vk::ImageSubresourceLayers = vk::ImageSubresourceLayers {
    aspect_mask: vk::ImageAspectFlags::COLOR,
    mip_level: 0,
    base_array_layer: 0,
    layer_count: 1,
};

pub struct Ps5Display {
    #[allow(dead_code)]
    entry: ash::Entry,
    #[allow(dead_code)]
    instance: ash::Instance,
    device: ash::Device,
    queue: vk::Queue,
    swapchain_fn: ash::khr::swapchain::Device,
    swapchain: vk::SwapchainKHR,
    swapchain_images: Vec<vk::Image>,
    extent: vk::Extent2D,
    command_buffer: vk::CommandBuffer,
    staging_buffer: vk::Buffer,
    staging_memory: vk::DeviceMemory,
    staging_ptr: *mut u8,
    frame_image: vk::Image,
    submit_fence: vk::Fence,
    acquire_fence: vk::Fence,
    presented: std::cell::Cell<u64>,
}

unsafe extern "system" {
    fn vkGetInstanceProcAddr(
        instance: vk::Instance,
        p_name: *const core::ffi::c_char,
    ) -> vk::PFN_vkVoidFunction;
}

fn find_memory(
    props: &vk::PhysicalDeviceMemoryProperties,
    bits: u32,
    flags: vk::MemoryPropertyFlags,
) -> Option<u32> {
    (0..props.memory_type_count).find(|&i| {
        bits & (1 << i) != 0 && props.memory_types[i as usize].property_flags.contains(flags)
    })
}

impl Ps5Display {
    pub unsafe fn new() -> Result<Self, String> {
        println!("[PS5] Creating display...");

        let entry = unsafe {
            ash::Entry::from_static_fn(ash::StaticFn {
                get_instance_proc_addr: vkGetInstanceProcAddr,
            })
        };

        let app_info = vk::ApplicationInfo::default()
            .application_name(c"Ruffle Flash PS5")
            .engine_name(c"Ruffle")
            .api_version(vk::API_VERSION_1_1);
        let extensions = [ash::khr::surface::NAME.as_ptr(), ash::khr::display::NAME.as_ptr()];
        let instance = unsafe {
            entry
                .create_instance(
                    &vk::InstanceCreateInfo::default()
                        .application_info(&app_info)
                        .enabled_extension_names(&extensions),
                    None,
                )
                .map_err(|e| format!("vkCreateInstance: {:?}", e))?
        };

        let display_fn = ash::khr::display::Instance::new(&entry, &instance);
        let surface_fn = ash::khr::surface::Instance::new(&entry, &instance);

        let physical_device = *unsafe { instance.enumerate_physical_devices() }
            .map_err(|e| format!("enumerate_physical_devices: {:?}", e))?
            .first()
            .ok_or("no physical device")?;
        let props = unsafe { instance.get_physical_device_properties(physical_device) };
        println!(
            "[PS5] GPU: {}",
            unsafe { CStr::from_ptr(props.device_name.as_ptr()) }.to_string_lossy()
        );

        // Reading a frame back is only fast from CPU-cached memory.
        let mem = unsafe { instance.get_physical_device_memory_properties(physical_device) };
        for i in 0..mem.memory_type_count as usize {
            let t = mem.memory_types[i];
            println!(
                "[PS5] memory type {}: heap {} ({} MB), {:?}",
                i,
                t.heap_index,
                mem.memory_heaps[t.heap_index as usize].size >> 20,
                t.property_flags
            );
        }

        let display = unsafe { display_fn.get_physical_device_display_properties(physical_device) }
            .map_err(|e| format!("get_display_properties: {:?}", e))?
            .first()
            .ok_or("no display")?
            .display;

        // The display's own mode: XPSemu presents at its visible region (3840x2160).
        let modes = unsafe { display_fn.get_display_mode_properties(physical_device, display) }
            .map_err(|e| format!("get_display_mode_properties: {:?}", e))?;
        let mode = modes
            .iter()
            .max_by_key(|m| {
                let r = m.parameters.visible_region;
                (r.width * r.height, m.parameters.refresh_rate)
            })
            .ok_or("no display mode")?;
        let extent = mode.parameters.visible_region;
        println!(
            "[PS5] Display mode: {}x{} @ {} mHz ({} modes)",
            extent.width,
            extent.height,
            mode.parameters.refresh_rate,
            modes.len()
        );

        let surface = unsafe {
            display_fn.create_display_plane_surface(
                &vk::DisplaySurfaceCreateInfoKHR::default()
                    .display_mode(mode.display_mode)
                    .plane_index(0)
                    .plane_stack_index(0)
                    .transform(vk::SurfaceTransformFlagsKHR::IDENTITY)
                    .global_alpha(1.0)
                    .alpha_mode(vk::DisplayPlaneAlphaFlagsKHR::OPAQUE)
                    .image_extent(extent),
                None,
            )
        }
        .map_err(|e| format!("create_display_plane_surface: {:?}", e))?;

        let queue_family = unsafe { instance.get_physical_device_queue_family_properties(physical_device) }
            .iter()
            .enumerate()
            .position(|(i, qf)| {
                qf.queue_flags.contains(vk::QueueFlags::GRAPHICS)
                    && unsafe {
                        surface_fn
                            .get_physical_device_surface_support(physical_device, i as u32, surface)
                            .unwrap_or(false)
                    }
            })
            .ok_or("no graphics queue that can present")? as u32;

        let device_extensions = [ash::khr::swapchain::NAME.as_ptr()];
        let queue_infos = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family)
            .queue_priorities(&[1.0])];
        let device = unsafe {
            instance.create_device(
                physical_device,
                &vk::DeviceCreateInfo::default()
                    .queue_create_infos(&queue_infos)
                    .enabled_extension_names(&device_extensions),
                None,
            )
        }
        .map_err(|e| format!("create_device: {:?}", e))?;
        let queue = unsafe { device.get_device_queue(queue_family, 0) };
        let swapchain_fn = ash::khr::swapchain::Device::new(&instance, &device);

        let caps = unsafe { surface_fn.get_physical_device_surface_capabilities(physical_device, surface) }
            .map_err(|e| format!("get_surface_capabilities: {:?}", e))?;
        let formats = unsafe { surface_fn.get_physical_device_surface_formats(physical_device, surface) }
            .map_err(|e| format!("get_surface_formats: {:?}", e))?;
        println!(
            "[PS5] Surface: images {}..{}, current extent {}x{}, usage {:?}, alpha {:?}, formats {:?}",
            caps.min_image_count,
            caps.max_image_count,
            caps.current_extent.width,
            caps.current_extent.height,
            caps.supported_usage_flags,
            caps.supported_composite_alpha,
            formats.iter().map(|f| f.format).collect::<Vec<_>>()
        );

        let format = formats
            .iter()
            .find(|f| f.format == vk::Format::B8G8R8A8_UNORM)
            .or_else(|| formats.first())
            .copied()
            .ok_or("no surface format")?;
        let swap_extent = if caps.current_extent.width != u32::MAX {
            caps.current_extent
        } else {
            extent
        };
        let mut image_count = caps.min_image_count.max(3);
        if caps.max_image_count != 0 {
            image_count = image_count.min(caps.max_image_count);
        }
        let composite_alpha = if caps
            .supported_composite_alpha
            .contains(vk::CompositeAlphaFlagsKHR::OPAQUE)
        {
            vk::CompositeAlphaFlagsKHR::OPAQUE
        } else {
            vk::CompositeAlphaFlagsKHR::from_raw(
                caps.supported_composite_alpha.as_raw() & caps.supported_composite_alpha.as_raw().wrapping_neg(),
            )
        };
        if !caps.supported_usage_flags.contains(vk::ImageUsageFlags::TRANSFER_DST) {
            return Err(format!(
                "swapchain images can't be blitted to (usage {:?})",
                caps.supported_usage_flags
            ));
        }

        let swapchain = unsafe {
            swapchain_fn.create_swapchain(
                &vk::SwapchainCreateInfoKHR::default()
                    .surface(surface)
                    .min_image_count(image_count)
                    .image_format(format.format)
                    .image_color_space(format.color_space)
                    .image_extent(swap_extent)
                    .image_array_layers(1)
                    .image_usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::COLOR_ATTACHMENT)
                    .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .pre_transform(caps.current_transform)
                    .composite_alpha(composite_alpha)
                    .present_mode(vk::PresentModeKHR::FIFO)
                    .clipped(true),
                None,
            )
        }
        .map_err(|e| format!("create_swapchain: {:?}", e))?;
        let swapchain_images = unsafe { swapchain_fn.get_swapchain_images(swapchain) }
            .map_err(|e| format!("get_swapchain_images: {:?}", e))?;
        println!(
            "[PS5] Swapchain: {} images {}x{} format {:?}",
            swapchain_images.len(),
            swap_extent.width,
            swap_extent.height,
            format.format
        );

        let pool = unsafe {
            device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(queue_family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )
        }
        .map_err(|e| format!("create_command_pool: {:?}", e))?;
        let command_buffer = unsafe {
            device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )
        }
        .map_err(|e| format!("allocate_command_buffers: {:?}", e))?[0];

        let mem_props = unsafe { instance.get_physical_device_memory_properties(physical_device) };

        // Host-visible staging buffer, mapped once.
        let size = (FRAME_WIDTH * FRAME_HEIGHT * 4) as vk::DeviceSize;
        let staging_buffer = unsafe {
            device.create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(size)
                    .usage(vk::BufferUsageFlags::TRANSFER_SRC)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )
        }
        .map_err(|e| format!("create_buffer: {:?}", e))?;
        let reqs = unsafe { device.get_buffer_memory_requirements(staging_buffer) };
        let mem_type = find_memory(
            &mem_props,
            reqs.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )
        .ok_or("no host-visible memory")?;
        let staging_memory = unsafe {
            device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(reqs.size)
                    .memory_type_index(mem_type),
                None,
            )
        }
        .map_err(|e| format!("allocate_memory (staging): {:?}", e))?;
        unsafe { device.bind_buffer_memory(staging_buffer, staging_memory, 0) }
            .map_err(|e| format!("bind_buffer_memory: {:?}", e))?;
        let staging_ptr = unsafe {
            device.map_memory(staging_memory, 0, size, vk::MemoryMapFlags::empty())
        }
        .map_err(|e| format!("map_memory: {:?}", e))? as *mut u8;

        // The frame in device memory, blitted (scaled) to the swapchain image.
        let frame_image = unsafe {
            device.create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(vk::Format::R8G8B8A8_UNORM)
                    .extent(vk::Extent3D {
                        width: FRAME_WIDTH,
                        height: FRAME_HEIGHT,
                        depth: 1,
                    })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::TRANSFER_SRC)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .initial_layout(vk::ImageLayout::UNDEFINED),
                None,
            )
        }
        .map_err(|e| format!("create_image: {:?}", e))?;
        let reqs = unsafe { device.get_image_memory_requirements(frame_image) };
        let mem_type = find_memory(&mem_props, reqs.memory_type_bits, vk::MemoryPropertyFlags::DEVICE_LOCAL)
            .or_else(|| find_memory(&mem_props, reqs.memory_type_bits, vk::MemoryPropertyFlags::empty()))
            .ok_or("no memory for the frame image")?;
        let frame_memory = unsafe {
            device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(reqs.size)
                    .memory_type_index(mem_type),
                None,
            )
        }
        .map_err(|e| format!("allocate_memory (frame): {:?}", e))?;
        unsafe { device.bind_image_memory(frame_image, frame_memory, 0) }
            .map_err(|e| format!("bind_image_memory: {:?}", e))?;

        let fence_info = vk::FenceCreateInfo::default();
        let submit_fence = unsafe { device.create_fence(&fence_info, None) }
            .map_err(|e| format!("create_fence: {:?}", e))?;
        let acquire_fence = unsafe { device.create_fence(&fence_info, None) }
            .map_err(|e| format!("create_fence: {:?}", e))?;

        println!("[PS5] Display ready");

        Ok(Ps5Display {
            entry,
            instance,
            device,
            queue,
            swapchain_fn,
            swapchain,
            swapchain_images,
            extent: swap_extent,
            command_buffer,
            staging_buffer,
            staging_memory,
            staging_ptr,
            frame_image,
            submit_fence,
            acquire_fence,
            presented: std::cell::Cell::new(0),
        })
    }

    /// Shows one RGBA8 frame of FRAME_WIDTH x FRAME_HEIGHT, scaled to the display.
    pub unsafe fn present_frame(&self, pixels: &[u8]) {
        let size = (FRAME_WIDTH * FRAME_HEIGHT * 4) as usize;
        if pixels.len() < size {
            return;
        }
        let _ = self.staging_memory;
        unsafe { std::ptr::copy_nonoverlapping(pixels.as_ptr(), self.staging_ptr, size) };

        let d = &self.device;
        let image_index = match unsafe {
            self.swapchain_fn.acquire_next_image(
                self.swapchain,
                u64::MAX,
                vk::Semaphore::null(),
                self.acquire_fence,
            )
        } {
            Ok((i, _)) => i,
            Err(e) => {
                println!("[PS5] acquire_next_image: {:?}", e);
                return;
            }
        };
        unsafe {
            d.wait_for_fences(&[self.acquire_fence], true, u64::MAX).ok();
            d.reset_fences(&[self.acquire_fence]).ok();
        }
        let target = self.swapchain_images[image_index as usize];
        let cb = self.command_buffer;

        let barrier = |image, old, new, src, dst| {
            vk::ImageMemoryBarrier::default()
                .image(image)
                .old_layout(old)
                .new_layout(new)
                .src_access_mask(src)
                .dst_access_mask(dst)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .subresource_range(SUBRESOURCE)
        };

        unsafe {
            d.reset_command_buffer(cb, vk::CommandBufferResetFlags::empty()).ok();
            d.begin_command_buffer(
                cb,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )
            .ok();

            d.cmd_pipeline_barrier(
                cb,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[
                    barrier(
                        self.frame_image,
                        vk::ImageLayout::UNDEFINED,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        vk::AccessFlags::empty(),
                        vk::AccessFlags::TRANSFER_WRITE,
                    ),
                    barrier(
                        target,
                        vk::ImageLayout::UNDEFINED,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        vk::AccessFlags::empty(),
                        vk::AccessFlags::TRANSFER_WRITE,
                    ),
                ],
            );

            d.cmd_copy_buffer_to_image(
                cb,
                self.staging_buffer,
                self.frame_image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[vk::BufferImageCopy {
                    buffer_offset: 0,
                    buffer_row_length: 0,
                    buffer_image_height: 0,
                    image_subresource: LAYERS,
                    image_offset: vk::Offset3D::default(),
                    image_extent: vk::Extent3D {
                        width: FRAME_WIDTH,
                        height: FRAME_HEIGHT,
                        depth: 1,
                    },
                }],
            );

            d.cmd_pipeline_barrier(
                cb,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier(
                    self.frame_image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    vk::AccessFlags::TRANSFER_WRITE,
                    vk::AccessFlags::TRANSFER_READ,
                )],
            );

            // RGBA source into the BGRA swapchain: blit converts the format.
            d.cmd_blit_image(
                cb,
                self.frame_image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                target,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[vk::ImageBlit {
                    src_subresource: LAYERS,
                    src_offsets: [
                        vk::Offset3D::default(),
                        vk::Offset3D {
                            x: FRAME_WIDTH as i32,
                            y: FRAME_HEIGHT as i32,
                            z: 1,
                        },
                    ],
                    dst_subresource: LAYERS,
                    dst_offsets: [
                        vk::Offset3D::default(),
                        vk::Offset3D {
                            x: self.extent.width as i32,
                            y: self.extent.height as i32,
                            z: 1,
                        },
                    ],
                }],
                vk::Filter::LINEAR,
            );

            d.cmd_pipeline_barrier(
                cb,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier(
                    target,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    vk::ImageLayout::PRESENT_SRC_KHR,
                    vk::AccessFlags::TRANSFER_WRITE,
                    vk::AccessFlags::empty(),
                )],
            );
            d.end_command_buffer(cb).ok();

            let cbs = [cb];
            let submit = vk::SubmitInfo::default().command_buffers(&cbs);
            if let Err(e) = d.queue_submit(self.queue, &[submit], self.submit_fence) {
                println!("[PS5] queue_submit: {:?}", e);
                return;
            }
            d.wait_for_fences(&[self.submit_fence], true, u64::MAX).ok();
            d.reset_fences(&[self.submit_fence]).ok();

            let swapchains = [self.swapchain];
            let indices = [image_index];
            if let Err(e) = self.swapchain_fn.queue_present(
                self.queue,
                &vk::PresentInfoKHR::default()
                    .swapchains(&swapchains)
                    .image_indices(&indices),
            ) {
                println!("[PS5] queue_present: {:?}", e);
            }
        }

        let n = self.presented.get() + 1;
        self.presented.set(n);
        if n == 1 || n % 600 == 0 {
            println!("[PS5] presented {} frames", n);
        }
    }
}
