# LFS — LYSOR File System 规范

## 设计目标



* 简洁：不搞日志、不搞碎片整理、不搞 ext4 那堆复杂特性

* 快速：extent 连续分配，顺序 IO 一次读满

* 安全：每 inode 带 CRC32 校验，权限位

* 干净：一个文件一个模块，不超过 300 行

## 磁盘布局



```
Block 0     Superblock (4KB)

Block 1     Block位图 (4KB = 32768 blocks = 128MB)

Block 2-5   Inode表 (64个inode \* 128字节 = 8KB)

Block 6+    数据块
```

## Superblock (128 字节有效)



| 偏移 | 大小 | 字段                  | 说明                |
| -- | -- | ------------------- | ----------------- |
| 0  | 4  | magic               | 0x4C465331 "LFS1" |
| 4  | 4  | version             | 1                 |
| 8  | 4  | block\_size         | 4096              |
| 12 | 4  | total\_blocks       | 总块数               |
| 16 | 4  | free\_blocks        | 空闲块               |
| 20 | 4  | inode\_count        | inode 总数          |
| 24 | 4  | free\_inodes        | 空闲 inode          |
| 28 | 4  | inode\_table\_start | inode 表起始块        |
| 32 | 4  | data\_start         | 数据区起始块            |
| 36 | 4  | checksum            | superblock CRC    |

## Inode (128 字节)



| 偏移 | 大小 | 字段          | 说明                |
| -- | -- | ----------- | ----------------- |
| 0  | 2  | mode        | 类型 + 权限 (unix 风格) |
| 2  | 2  | uid         | 属主                |
| 4  | 4  | size        | 文件大小 (字节)         |
| 8  | 4  | blocks      | 占用块数              |
| 12 | 48 | direct\[12] | 12 个直接块指针         |
| 60 | 4  | indirect    | 间接块指针 (预留)        |
| 64 | 4  | ctime       | 创建时间              |
| 68 | 4  | mtime       | 修改时间              |
| 72 | 4  | checksum    | inode CRC32       |

**最大文件**：12 \* 4KB = 48KB（直接块）。后续扩展间接块到～16MB。

## 目录项 (44 字节)



| 偏移 | 大小 | 字段                         |
| -- | -- | -------------------------- |
| 0  | 4  | inode 号                    |
| 4  | 2  | name\_len                  |
| 6  | 1  | file\_type (0=file, 1=dir) |
| 7  | 1  | pad                        |
| 8  | 28 | name (最长 28 字符)            |

## 为什么不要日志？

LFS 定位是**内存盘 / 嵌入式系统盘**，掉电即重置。日志 (JBD) 是为了掉电恢复，我们不需要。

需要持久化时再叠一层块设备快照即可。

## 后续扩展路线



1. 间接块 → 支持 >48KB 文件

2. 符号链接

3. 硬链接

4. CRC32 数据块校验

5. 挂载到真实块设备（ATA/SATA）